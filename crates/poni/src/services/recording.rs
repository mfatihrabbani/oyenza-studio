//! Recording a meeting's sound to an .m4a file: the default microphone
//! (left) and what the PC plays, through WASAPI loopback (right), mixed
//! and resampled by `poni_core::recording` and encoded to AAC by Media
//! Foundation's own encoder (no codec of our own). One thread while it
//! records, none otherwise. It stops when asked, when a device goes away
//! (the file keeps what came before) or when the disk runs nearly full.

use crate::events::{AppEvent, EventSink, RecordingEvent};
use poni_core::recording::{self as model, Mixer, Resampler, AAC_BYTES_PER_SECOND, RATE};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::core::HSTRING;
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, EDataFlow, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
};
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFByteStream, IMFSinkWriter, MFAudioFormat_AAC, MFAudioFormat_PCM, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
    MFCreateSinkWriterFromURL, MFMediaType_Audio, MFShutdown, MFStartup, MFSTARTUP_FULL, MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE,
    MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_VERSION,
};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED};

/// How often the devices are read: their buffers hold two seconds, so this
/// is only about keeping the file's pieces small.
const TICK: Duration = Duration::from_millis(40);
/// How often the free space is checked while recording.
const DISK_EVERY: Duration = Duration::from_secs(30);
/// What the devices may buffer (100-ns units): two seconds.
const BUFFER_HNS: i64 = 20_000_000;
const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// A recording under way. Dropping it stops it; the file is finished on
/// the recording's own thread, which then sends `AppEvent::Recording`.
pub struct Recorder {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Recorder {
    /// Start recording to `path` (its folder is made if needed). An error
    /// is a sentence for the island.
    pub fn start(path: PathBuf, sink: EventSink) -> Result<Recorder, String> {
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        std::fs::create_dir_all(&dir).map_err(|e| format!("Can't make {} ({e})", dir.display()))?;
        if free_bytes(&dir).is_some_and(|f| !model::can_start(f)) {
            return Err("Less than 500 MB free on that disk".into());
        }
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("poni-recording".into())
            .spawn(move || run(path, flag, sink))
            .map_err(|e| format!("No recording thread ({e})"))?;
        Ok(Recorder { stop, thread: Some(thread) })
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Finishing the file takes a moment; the thread reports when done.
        drop(self.thread.take());
    }
}

fn run(path: PathBuf, stop: Arc<AtomicBool>, sink: EventSink) {
    let event = unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let started = MFStartup(MF_VERSION, MFSTARTUP_FULL);
        let out = match started {
            Ok(()) => record(&path, &stop),
            Err(e) => Err(format!("Media Foundation didn't start ({e})")),
        };
        if started.is_ok() {
            let _ = MFShutdown();
        }
        CoUninitialize();
        match out {
            Ok((secs, low_disk, why)) => RecordingEvent::Saved { path: path.to_string_lossy().into_owned(), secs, low_disk, stopped_by: why },
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                RecordingEvent::Failed(e)
            }
        }
    };
    sink.send(AppEvent::Recording(event));
}

/// One device being read.
struct Source {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    channels: usize,
    float: bool,
    resampler: Resampler,
}

/// Record until asked to stop. Returns the seconds written, whether the
/// disk ran low, and what stopped it early, if anything did.
unsafe fn record(path: &Path, stop: &AtomicBool) -> Result<(f64, bool, Option<String>), String> {
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| format!("No sound devices ({e})"))?;
    let mut mic = open(&enumerator, eCapture, false).map_err(|e| crate::pwarn!("recording: no microphone ({e})")).ok();
    let mut others = open(&enumerator, eRender, true).map_err(|e| crate::pwarn!("recording: no playback to record ({e})")).ok();
    if mic.is_none() && others.is_none() {
        return Err("No microphone or speakers to record".into());
    }
    let mut enc = Encoder::new(path).map_err(|e| format!("Can't write the file ({e})"))?;
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut mixer = Mixer::default();
    let mut buf = Vec::new();
    let started = Instant::now();
    let mut disk_at = Instant::now();
    let (mut low_disk, mut why) = (false, None);
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(TICK);
        let step = (|| -> windows::core::Result<()> {
            if let Some(s) = mic.as_mut() {
                buf.clear();
                drain(s, &mut buf)?;
                mixer.push_mic(&buf);
            }
            if let Some(s) = others.as_mut() {
                buf.clear();
                drain(s, &mut buf)?;
                mixer.push_others(&buf);
            }
            // The microphone keeps time; without one, the clock does.
            let frames = if mic.is_some() {
                mixer.mic_queued()
            } else {
                let due = (started.elapsed().as_secs_f64() * RATE as f64) as u64;
                due.saturating_sub(enc.frames) as usize
            };
            enc.write(&mixer.take(frames))
        })();
        if let Err(e) = step {
            // A headset unplugged, a device gone: keep what's recorded.
            crate::pwarn!("recording: stopped early ({e})");
            why = Some("A sound device went away".to_string());
            break;
        }
        if disk_at.elapsed() >= DISK_EVERY {
            disk_at = Instant::now();
            if free_bytes(&dir).is_some_and(model::must_stop) {
                low_disk = true;
                break;
            }
        }
    }
    for s in [&mic, &others].into_iter().flatten() {
        let _ = s.client.Stop();
    }
    let secs = enc.frames as f64 / RATE as f64;
    enc.writer.Finalize().map_err(|e| format!("Couldn't finish the file ({e})"))?;
    Ok((secs, low_disk, why))
}

/// The default microphone (`eCapture`) or speakers (`eRender`, read through
/// loopback), started.
unsafe fn open(enumerator: &IMMDeviceEnumerator, flow: EDataFlow, loopback: bool) -> windows::core::Result<Source> {
    let device = enumerator.GetDefaultAudioEndpoint(flow, eConsole)?;
    let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
    let fmt = client.GetMixFormat()?;
    let tag = std::ptr::addr_of!((*fmt).wFormatTag).read_unaligned();
    let channels = std::ptr::addr_of!((*fmt).nChannels).read_unaligned() as usize;
    let rate = std::ptr::addr_of!((*fmt).nSamplesPerSec).read_unaligned();
    let bits = std::ptr::addr_of!((*fmt).wBitsPerSample).read_unaligned();
    // WAVEFORMATEXTENSIBLE's sub-format GUID starts 24 bytes in; its first
    // field is the format tag it stands for.
    let sub = if tag == WAVE_FORMAT_EXTENSIBLE { ((fmt as *const u8).add(24) as *const u32).read_unaligned() as u16 } else { tag };
    let float = sub == WAVE_FORMAT_IEEE_FLOAT && bits == 32;
    let pcm16 = sub == WAVE_FORMAT_PCM && bits == 16;
    let init = if float || pcm16 {
        let flags = if loopback { AUDCLNT_STREAMFLAGS_LOOPBACK } else { 0 };
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, BUFFER_HNS, 0, fmt, None)
    } else {
        Err(windows::core::Error::new(E_FAIL, format!("a {bits}-bit format {sub} it can't read")))
    };
    CoTaskMemFree(Some(fmt as *const _));
    init?;
    let capture: IAudioCaptureClient = client.GetService()?;
    client.Start()?;
    Ok(Source { client, capture, channels: channels.max(1), float, resampler: Resampler::new(rate, RATE) })
}

/// Everything the device has, as one channel at `RATE`, onto `out`.
unsafe fn drain(s: &mut Source, out: &mut Vec<f32>) -> windows::core::Result<()> {
    loop {
        if s.capture.GetNextPacketSize()? == 0 {
            return Ok(());
        }
        let mut data: *mut u8 = std::ptr::null_mut();
        let (mut frames, mut flags) = (0u32, 0u32);
        s.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
        let n = frames as usize * s.channels;
        let mono = if data.is_null() || flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
            vec![0.0; frames as usize]
        } else if s.float {
            model::downmix(std::slice::from_raw_parts(data as *const f32, n), s.channels)
        } else {
            model::downmix_i16(std::slice::from_raw_parts(data as *const i16, n), s.channels)
        };
        s.capture.ReleaseBuffer(frames)?;
        s.resampler.push(&mono, out);
    }
}

/// Media Foundation's sink writer: 16-bit stereo PCM in, AAC in an .m4a out.
struct Encoder {
    writer: IMFSinkWriter,
    stream: u32,
    /// Frames written so far.
    frames: u64,
}

impl Encoder {
    unsafe fn new(path: &Path) -> windows::core::Result<Self> {
        let writer = MFCreateSinkWriterFromURL(&HSTRING::from(path.as_os_str()), None::<&IMFByteStream>, None::<&IMFAttributes>)?;
        let out = MFCreateMediaType()?;
        out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        out.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
        out.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        out.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, RATE)?;
        out.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
        out.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, AAC_BYTES_PER_SECOND)?;
        let stream = writer.AddStream(&out)?;
        let input = MFCreateMediaType()?;
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        input.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        input.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, RATE)?;
        input.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
        input.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
        input.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, RATE * 4)?;
        writer.SetInputMediaType(stream, &input, None::<&IMFAttributes>)?;
        writer.BeginWriting()?;
        Ok(Self { writer, stream, frames: 0 })
    }

    /// Interleaved stereo samples onto the file.
    unsafe fn write(&mut self, pcm: &[i16]) -> windows::core::Result<()> {
        if pcm.is_empty() {
            return Ok(());
        }
        let bytes = (pcm.len() * 2) as u32;
        let buffer = MFCreateMemoryBuffer(bytes)?;
        let mut p: *mut u8 = std::ptr::null_mut();
        buffer.Lock(&mut p, None, None)?;
        std::ptr::copy_nonoverlapping(pcm.as_ptr() as *const u8, p, bytes as usize);
        buffer.Unlock()?;
        buffer.SetCurrentLength(bytes)?;
        let sample = MFCreateSample()?;
        sample.AddBuffer(&buffer)?;
        let frames = (pcm.len() / 2) as u64;
        // Times in 100-ns units.
        sample.SetSampleTime((self.frames * 10_000_000 / RATE as u64) as i64)?;
        sample.SetSampleDuration((frames * 10_000_000 / RATE as u64) as i64)?;
        self.writer.WriteSample(self.stream, &sample)?;
        self.frames += frames;
        Ok(())
    }
}

/// Free bytes on the disk holding `dir` (for this user).
fn free_bytes(dir: &Path) -> Option<u64> {
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(dir.as_os_str()), Some(&mut free), None, None) }.ok()?;
    Some(free)
}
