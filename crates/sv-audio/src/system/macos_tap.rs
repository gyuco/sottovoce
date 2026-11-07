//! Cattura dell'audio di sistema su macOS 14.4+ con un Core Audio process tap.
//!
//! Flusso:
//! 1. `CATapDescription` globale mono che esclude il processo di Sottovoce.
//! 2. `AudioHardwareCreateProcessTap` → tap.
//! 3. Aggregate device privato con il tap (e il device di output di default
//!    come clock principale).
//! 4. IOProc a blocco sull'aggregate: i campioni del tap finiscono nel ring buffer.
//!
//! Permesso richiesto: "Registrazione audio di sistema" (NSAudioCaptureUsageDescription),
//! non "Registrazione schermo". Senza permesso il tap restituisce silenzio.

use std::cell::RefCell;
use std::ffi::CStr;
use std::mem::size_of;
use std::ptr::{NonNull, null};

use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_audio::{
    AudioDeviceCreateIOProcIDWithBlock, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID,
    AudioDeviceStart, AudioDeviceStop, AudioHardwareCreateAggregateDevice,
    AudioHardwareCreateProcessTap, AudioHardwareDestroyAggregateDevice,
    AudioHardwareDestroyProcessTap, AudioObjectGetPropertyData, AudioObjectID,
    AudioObjectPropertyAddress, CATapDescription, CATapMuteBehavior,
    kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceMainSubDeviceKey,
    kAudioAggregateDeviceNameKey, kAudioAggregateDeviceSubDeviceListKey,
    kAudioAggregateDeviceTapAutoStartKey, kAudioAggregateDeviceTapListKey,
    kAudioAggregateDeviceUIDKey, kAudioDevicePropertyDeviceUID,
    kAudioHardwarePropertyDefaultSystemOutputDevice,
    kAudioHardwarePropertyTranslatePIDToProcessObject, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioSubDeviceUIDKey,
    kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey, kAudioTapPropertyFormat,
};
use objc2_core_audio_types::{AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp};
use objc2_core_foundation::CFDictionary;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, NSUUID};
use ringbuf::HeapProd;

use crate::error::AudioError;
use crate::source::{OpenedSource, push_converted, ring};

const SYSTEM_OBJECT: AudioObjectID = kAudioObjectSystemObject as AudioObjectID;

fn check(call: &'static str, status: i32) -> Result<(), AudioError> {
    if status == 0 {
        Ok(())
    } else {
        Err(AudioError::CoreAudio { call, status })
    }
}

fn address(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// Legge una proprietà di dimensione fissa `T`.
///
/// # Safety
/// `T` deve corrispondere al tipo della proprietà e il qualifier (se presente)
/// deve essere quello atteso dalla proprietà.
unsafe fn get_property<T: Copy>(
    object: AudioObjectID,
    selector: u32,
    qualifier: Option<&[u8]>,
    init: T,
) -> Result<T, AudioError> {
    let addr = address(selector);
    let mut value = init;
    let mut size = size_of::<T>() as u32;
    let (q_size, q_ptr) = qualifier.map_or((0, null()), |q| (q.len() as u32, q.as_ptr().cast()));
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&addr),
            q_size,
            q_ptr,
            NonNull::from(&mut size),
            NonNull::from(&mut value).cast(),
        )
    };
    check("AudioObjectGetPropertyData", status)?;
    Ok(value)
}

fn nsstring(key: &CStr) -> Retained<NSString> {
    NSString::from_str(key.to_str().expect("chiave Core Audio ASCII"))
}

fn any<T: objc2::Message>(obj: Retained<T>) -> Retained<AnyObject> {
    // SAFETY: ogni oggetto Objective-C è un AnyObject.
    unsafe { Retained::cast_unchecked(obj) }
}

fn dict(pairs: Vec<(&CStr, Retained<AnyObject>)>) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<Retained<NSString>> = pairs.iter().map(|(k, _)| nsstring(k)).collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let values: Vec<Retained<AnyObject>> = pairs.into_iter().map(|(_, v)| v).collect();
    NSDictionary::from_retained_objects(&key_refs, &values)
}

/// AudioObjectID del processo corrente (da escludere dal tap), se disponibile.
fn own_process_object() -> Option<AudioObjectID> {
    let pid = std::process::id() as i32;
    // SAFETY: TranslatePIDToProcessObject vuole un pid_t come qualifier e restituisce un AudioObjectID.
    let obj = unsafe {
        get_property::<AudioObjectID>(
            SYSTEM_OBJECT,
            kAudioHardwarePropertyTranslatePIDToProcessObject,
            Some(&pid.to_ne_bytes()),
            0,
        )
    }
    .ok()?;
    (obj != 0).then_some(obj)
}

/// UID del device di output di default (clock principale dell'aggregate).
fn default_output_uid() -> Option<Retained<NSString>> {
    // SAFETY: la proprietà restituisce un AudioObjectID.
    let dev = unsafe {
        get_property::<AudioObjectID>(
            SYSTEM_OBJECT,
            kAudioHardwarePropertyDefaultSystemOutputDevice,
            None,
            0,
        )
    }
    .ok()?;
    // SAFETY: DeviceUID restituisce un CFStringRef con +1 di retain (toll-free bridged a NSString).
    let ptr = unsafe {
        get_property::<*mut NSString>(
            dev,
            kAudioDevicePropertyDeviceUID,
            None,
            std::ptr::null_mut(),
        )
    }
    .ok()?;
    // SAFETY: prendiamo possesso del retain ricevuto.
    unsafe { Retained::from_raw(ptr) }
}

type IoBlock = RcBlock<
    dyn Fn(
        NonNull<AudioTimeStamp>,
        NonNull<AudioBufferList>,
        NonNull<AudioTimeStamp>,
        NonNull<AudioBufferList>,
        NonNull<AudioTimeStamp>,
    ),
>;

/// Risorse Core Audio della sessione; il drop le rilascia in ordine inverso.
struct TapGuard {
    tap_id: AudioObjectID,
    aggregate_id: AudioObjectID,
    proc_id: AudioDeviceIOProcID,
    started: bool,
    _block: Option<IoBlock>,
}

impl Drop for TapGuard {
    fn drop(&mut self) {
        // SAFETY: gli ID sono stati creati da noi e vengono distrutti una sola volta.
        unsafe {
            if self.started {
                AudioDeviceStop(self.aggregate_id, self.proc_id);
            }
            if self.proc_id.is_some() {
                AudioDeviceDestroyIOProcID(self.aggregate_id, self.proc_id);
            }
            if self.aggregate_id != 0 {
                AudioHardwareDestroyAggregateDevice(self.aggregate_id);
            }
            if self.tap_id != 0 {
                AudioHardwareDestroyProcessTap(self.tap_id);
            }
        }
    }
}

pub(crate) fn open() -> Result<OpenedSource, AudioError> {
    let mut guard = TapGuard {
        tap_id: 0,
        aggregate_id: 0,
        proc_id: None,
        started: false,
        _block: None,
    };

    // 1. Descrizione del tap: tutto l'audio di sistema in mono, tranne il nostro processo.
    let excluded: Vec<Retained<NSNumber>> = own_process_object()
        .map(|id| vec![NSNumber::new_u32(id)])
        .unwrap_or_default();
    let excluded = NSArray::from_retained_slice(&excluded);
    // SAFETY: API ObjC standard; l'array contiene NSNumber di AudioObjectID.
    let desc = unsafe {
        let d = CATapDescription::initMonoGlobalTapButExcludeProcesses(
            CATapDescription::alloc(),
            &excluded,
        );
        d.setName(&NSString::from_str("Sottovoce"));
        d.setPrivate(true);
        d.setMuteBehavior(CATapMuteBehavior::Unmuted);
        d
    };

    // 2. Tap.
    // SAFETY: descrizione valida, puntatore di output valido.
    check("AudioHardwareCreateProcessTap", unsafe {
        AudioHardwareCreateProcessTap(Some(&desc), &mut guard.tap_id)
    })?;

    // SAFETY: kAudioTapPropertyFormat restituisce un AudioStreamBasicDescription.
    let format = unsafe {
        get_property::<AudioStreamBasicDescription>(
            guard.tap_id,
            kAudioTapPropertyFormat,
            None,
            std::mem::zeroed(),
        )
    }?;
    let sample_rate = format.mSampleRate as u32;
    let channels = format.mChannelsPerFrame.max(1) as usize;
    // SAFETY: getter ObjC.
    let tap_uid = unsafe { desc.UUID().UUIDString() };

    // 3. Aggregate device privato.
    let tap_entry = dict(vec![
        (kAudioSubTapUIDKey, any(tap_uid)),
        (
            kAudioSubTapDriftCompensationKey,
            any(NSNumber::new_bool(true)),
        ),
    ]);
    let mut entries = vec![
        (
            kAudioAggregateDeviceNameKey,
            any(NSString::from_str("Sottovoce System Tap")),
        ),
        (kAudioAggregateDeviceUIDKey, any(NSUUID::new().UUIDString())),
        (
            kAudioAggregateDeviceIsPrivateKey,
            any(NSNumber::new_bool(true)),
        ),
        (
            kAudioAggregateDeviceTapAutoStartKey,
            any(NSNumber::new_bool(true)),
        ),
        (
            kAudioAggregateDeviceTapListKey,
            any(NSArray::from_retained_slice(&[tap_entry])),
        ),
    ];
    if let Some(out_uid) = default_output_uid() {
        let sub = dict(vec![(kAudioSubDeviceUIDKey, any(out_uid.clone()))]);
        entries.push((kAudioAggregateDeviceMainSubDeviceKey, any(out_uid)));
        entries.push((
            kAudioAggregateDeviceSubDeviceListKey,
            any(NSArray::from_retained_slice(&[sub])),
        ));
    }
    let agg_desc = dict(entries);
    // SAFETY: NSDictionary è toll-free bridged a CFDictionary.
    let cf_desc: &CFDictionary = unsafe { &*(Retained::as_ptr(&agg_desc).cast::<CFDictionary>()) };
    check("AudioHardwareCreateAggregateDevice", unsafe {
        AudioHardwareCreateAggregateDevice(cf_desc, NonNull::from(&mut guard.aggregate_id))
    })?;

    // 4. IOProc.
    let (prod, consumer) = ring(sample_rate, channels);
    let prod = RefCell::new(prod);
    let block: IoBlock = RcBlock::new(
        move |_now: NonNull<AudioTimeStamp>,
              input: NonNull<AudioBufferList>,
              _in_time: NonNull<AudioTimeStamp>,
              _out: NonNull<AudioBufferList>,
              _out_time: NonNull<AudioTimeStamp>| {
            // SAFETY: Core Audio garantisce la validità della lista per la durata della chiamata.
            unsafe { on_io(&prod, input, channels) };
        },
    );
    let block_ptr = (&*block as *const block2::DynBlock<_>).cast_mut();
    check("AudioDeviceCreateIOProcIDWithBlock", unsafe {
        AudioDeviceCreateIOProcIDWithBlock(
            NonNull::from(&mut guard.proc_id),
            guard.aggregate_id,
            None,
            block_ptr,
        )
    })?;
    guard._block = Some(block);

    check("AudioDeviceStart", unsafe {
        AudioDeviceStart(guard.aggregate_id, guard.proc_id)
    })?;
    guard.started = true;

    Ok(OpenedSource {
        sample_rate,
        channels,
        consumer,
        _guard: Box::new(guard),
    })
}

/// Callback realtime: copia il buffer del tap nel ring buffer.
///
/// Gli stream del tap seguono quelli dei sub-device nell'aggregate, quindi il
/// tap è l'ultimo buffer della lista (il device di output di solito non ha input).
///
/// # Safety
/// `input` deve puntare a una AudioBufferList valida.
unsafe fn on_io(prod: &RefCell<HeapProd<f32>>, input: NonNull<AudioBufferList>, channels: usize) {
    let list = unsafe { input.as_ref() };
    let n = list.mNumberBuffers as usize;
    if n == 0 {
        return;
    }
    let buffers = unsafe { std::slice::from_raw_parts(list.mBuffers.as_ptr(), n) };
    let buf = &buffers[n - 1];
    if buf.mData.is_null() || buf.mNumberChannels as usize != channels {
        return;
    }
    let len = buf.mDataByteSize as usize / size_of::<f32>();
    let data = unsafe { std::slice::from_raw_parts(buf.mData.cast::<f32>().cast_const(), len) };
    if let Ok(mut p) = prod.try_borrow_mut() {
        push_converted(&mut p, data, |s| s);
    }
}
