#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;
    use std::thread;
    use std::time::Duration;

    #[allow(dead_code)]
    #[repr(C)]
    struct WaveFormatEx {
        format_tag: u16,
        channels: u16,
        samples_per_sec: u32,
        avg_bytes_per_sec: u32,
        block_align: u16,
        bits_per_sample: u16,
        cb_size: u16,
    }

    #[allow(dead_code)]
    #[repr(C)]
    struct WaveHdr {
        data: *mut u8,
        buffer_length: u32,
        bytes_recorded: u32,
        user: usize,
        flags: u32,
        loops: u32,
        next: *mut WaveHdr,
        reserved: usize,
    }

    const WAVE_FORMAT_PCM: u16 = 1;
    const WAVE_MAPPER: u32 = u32::MAX;
    const WHDR_DONE: u32 = 0x0000_0001;
    const NUM_BUFS: usize = 6;

    #[link(name = "winmm")]
    extern "system" {
        fn waveOutOpen(
            phwo: *mut *mut c_void,
            device_id: u32,
            pwfx: *const WaveFormatEx,
            callback: usize,
            instance: usize,
            fdw_open: u32,
        ) -> u32;
        fn waveOutPrepareHeader(hwo: *mut c_void, pwh: *mut WaveHdr, cbwh: u32) -> u32;
        fn waveOutUnprepareHeader(hwo: *mut c_void, pwh: *mut WaveHdr, cbwh: u32) -> u32;
        fn waveOutWrite(hwo: *mut c_void, pwh: *mut WaveHdr, cbwh: u32) -> u32;
        fn waveOutReset(hwo: *mut c_void) -> u32;
        fn waveOutClose(hwo: *mut c_void) -> u32;
    }

    struct Slot {
        hdr: Box<WaveHdr>,
        buf: Vec<u8>,
        in_flight: bool,
    }

    pub struct NativeAudioSink {
        handle: *mut c_void,
        slots: Vec<Slot>,
        next_slot: usize,
    }

    unsafe impl Send for NativeAudioSink {}

    impl NativeAudioSink {
        pub fn try_open(sample_rate: u32, channels: u16) -> Option<Self> {
            let block_align = channels * 2;
            let fmt = WaveFormatEx {
                format_tag: WAVE_FORMAT_PCM,
                channels,
                samples_per_sec: sample_rate,
                avg_bytes_per_sec: sample_rate * (block_align as u32),
                block_align,
                bits_per_sample: 16,
                cb_size: 0,
            };
            let mut handle: *mut c_void = std::ptr::null_mut();
            let res = unsafe { waveOutOpen(&mut handle, WAVE_MAPPER, &fmt, 0, 0, 0) };
            if res != 0 || handle.is_null() {
                return None;
            }
            let mut slots = Vec::with_capacity(NUM_BUFS);
            for _ in 0..NUM_BUFS {
                slots.push(Slot {
                    hdr: Box::new(WaveHdr {
                        data: std::ptr::null_mut(),
                        buffer_length: 0,
                        bytes_recorded: 0,
                        user: 0,
                        flags: 0,
                        loops: 0,
                        next: std::ptr::null_mut(),
                        reserved: 0,
                    }),
                    buf: Vec::new(),
                    in_flight: false,
                });
            }
            Some(Self {
                handle,
                slots,
                next_slot: 0,
            })
        }

        pub fn write_pcm(&mut self, samples: &[i16]) {
            if samples.is_empty() {
                return;
            }
            let idx = self.next_slot % self.slots.len();
            self.next_slot = (idx + 1) % self.slots.len();
            let slot = &mut self.slots[idx];
            if slot.in_flight {
                let mut spins = 0u32;
                while (slot.hdr.flags & WHDR_DONE) == 0 && spins < 200 {
                    thread::sleep(Duration::from_millis(2));
                    spins += 1;
                }
                unsafe {
                    waveOutUnprepareHeader(
                        self.handle,
                        slot.hdr.as_mut(),
                        std::mem::size_of::<WaveHdr>() as u32,
                    );
                }
                slot.in_flight = false;
            }
            slot.buf.clear();
            slot.buf.reserve(samples.len() * 2);
            for &s in samples {
                slot.buf.extend_from_slice(&s.to_le_bytes());
            }
            slot.hdr.data = slot.buf.as_mut_ptr();
            slot.hdr.buffer_length = slot.buf.len() as u32;
            slot.hdr.flags = 0;
            unsafe {
                let hdr_sz = std::mem::size_of::<WaveHdr>() as u32;
                if waveOutPrepareHeader(self.handle, slot.hdr.as_mut(), hdr_sz) == 0 {
                    if waveOutWrite(self.handle, slot.hdr.as_mut(), hdr_sz) == 0 {
                        slot.in_flight = true;
                    } else {
                        waveOutUnprepareHeader(self.handle, slot.hdr.as_mut(), hdr_sz);
                    }
                }
            }
        }

        pub fn flush(&mut self) {
            unsafe {
                waveOutReset(self.handle);
                let hdr_sz = std::mem::size_of::<WaveHdr>() as u32;
                for slot in &mut self.slots {
                    if slot.in_flight {
                        waveOutUnprepareHeader(self.handle, slot.hdr.as_mut(), hdr_sz);
                        slot.in_flight = false;
                    }
                }
            }
        }
    }

    impl Drop for NativeAudioSink {
        fn drop(&mut self) {
            self.flush();
            unsafe {
                waveOutClose(self.handle);
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::collections::VecDeque;
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    #[allow(dead_code)]
    #[repr(C)]
    struct AudioStreamBasicDescription {
        sample_rate: f64,
        format_id: u32,
        format_flags: u32,
        bytes_per_packet: u32,
        frames_per_packet: u32,
        bytes_per_frame: u32,
        channels_per_frame: u32,
        bits_per_channel: u32,
        reserved: u32,
    }

    #[allow(dead_code)]
    #[repr(C)]
    struct AudioQueueBuffer {
        audio_data_bytes_capacity: u32,
        audio_data: *mut c_void,
        audio_data_byte_size: u32,
        user_data: *mut c_void,
        packet_description_capacity: u32,
        packet_descriptions: *mut c_void,
        packet_description_count: u32,
    }

    const K_AUDIO_FORMAT_LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");
    const K_LINEAR_PCM_SIGNED_PACKED: u32 = 4 | 8;
    const NUM_BUFS: usize = 6;
    const BUF_BYTES: u32 = 32768;

    #[link(name = "AudioToolbox", kind = "framework")]
    extern "C" {
        fn AudioQueueNewOutput(
            in_format: *const AudioStreamBasicDescription,
            in_callback_proc: extern "C" fn(*mut c_void, *mut c_void, *mut AudioQueueBuffer),
            in_user_data: *mut c_void,
            in_callback_run_loop: *mut c_void,
            in_callback_run_loop_mode: *mut c_void,
            in_flags: u32,
            out_aq: *mut *mut c_void,
        ) -> i32;
        fn AudioQueueAllocateBuffer(
            in_aq: *mut c_void,
            in_buffer_byte_size: u32,
            out_buffer: *mut *mut AudioQueueBuffer,
        ) -> i32;
        fn AudioQueueEnqueueBuffer(
            in_aq: *mut c_void,
            in_buffer: *mut AudioQueueBuffer,
            in_num_packet_descs: u32,
            in_packet_descs: *const c_void,
        ) -> i32;
        fn AudioQueueStart(in_aq: *mut c_void, in_start_time: *const c_void) -> i32;
        fn AudioQueueReset(in_aq: *mut c_void) -> i32;
        fn AudioQueueStop(in_aq: *mut c_void, in_immediate: u8) -> i32;
        fn AudioQueueDispose(in_aq: *mut c_void, in_immediate: u8) -> i32;
    }

    struct CallbackPool {
        free_bufs: Mutex<VecDeque<usize>>,
        ready: Condvar,
    }

    extern "C" fn aq_output_cb(
        user_data: *mut c_void,
        _queue: *mut c_void,
        buf: *mut AudioQueueBuffer,
    ) {
        if user_data.is_null() || buf.is_null() {
            return;
        }
        let pool = unsafe { &*(user_data as *const CallbackPool) };
        if let Ok(mut guard) = pool.free_bufs.lock() {
            guard.push_back(buf as usize);
            pool.ready.notify_one();
        }
    }

    pub struct NativeAudioSink {
        queue: *mut c_void,
        pool_ptr: *mut CallbackPool,
    }

    unsafe impl Send for NativeAudioSink {}

    impl NativeAudioSink {
        pub fn try_open(sample_rate: u32, channels: u16) -> Option<Self> {
            let bpf = (channels as u32) * 2;
            let desc = AudioStreamBasicDescription {
                sample_rate: sample_rate as f64,
                format_id: K_AUDIO_FORMAT_LINEAR_PCM,
                format_flags: K_LINEAR_PCM_SIGNED_PACKED,
                bytes_per_packet: bpf,
                frames_per_packet: 1,
                bytes_per_frame: bpf,
                channels_per_frame: channels as u32,
                bits_per_channel: 16,
                reserved: 0,
            };
            let pool = Box::new(CallbackPool {
                free_bufs: Mutex::new(VecDeque::with_capacity(NUM_BUFS)),
                ready: Condvar::new(),
            });
            let pool_ptr = Box::into_raw(pool);
            let mut queue: *mut c_void = ptr::null_mut();
            let status = unsafe {
                AudioQueueNewOutput(
                    &desc,
                    aq_output_cb,
                    pool_ptr as *mut c_void,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    0,
                    &mut queue,
                )
            };
            if status != 0 || queue.is_null() {
                unsafe {
                    drop(Box::from_raw(pool_ptr));
                }
                return None;
            }
            let mut allocated = 0usize;
            for _ in 0..NUM_BUFS {
                let mut buf: *mut AudioQueueBuffer = ptr::null_mut();
                let st = unsafe { AudioQueueAllocateBuffer(queue, BUF_BYTES, &mut buf) };
                if st == 0 && !buf.is_null() {
                    let pool_ref = unsafe { &*pool_ptr };
                    pool_ref.free_bufs.lock().unwrap().push_back(buf as usize);
                    allocated += 1;
                }
            }
            if allocated == 0 {
                unsafe {
                    AudioQueueDispose(queue, 1);
                    drop(Box::from_raw(pool_ptr));
                }
                return None;
            }
            let start_st = unsafe { AudioQueueStart(queue, ptr::null()) };
            if start_st != 0 {
                unsafe {
                    AudioQueueDispose(queue, 1);
                    drop(Box::from_raw(pool_ptr));
                }
                return None;
            }
            Some(Self { queue, pool_ptr })
        }

        pub fn write_pcm(&mut self, samples: &[i16]) {
            if samples.is_empty() {
                return;
            }
            let max_samples = (BUF_BYTES as usize) / 2;
            let pool = unsafe { &*self.pool_ptr };
            for chunk in samples.chunks(max_samples) {
                let mut guard = pool.free_bufs.lock().unwrap();
                if guard.is_empty() {
                    let (g, _) = pool
                        .ready
                        .wait_timeout(guard, Duration::from_millis(250))
                        .unwrap();
                    guard = g;
                }
                let Some(buf_addr) = guard.pop_front() else {
                    continue;
                };
                drop(guard);
                let buf = buf_addr as *mut AudioQueueBuffer;
                let byte_len = chunk.len() * 2;
                unsafe {
                    let cap = (*buf).audio_data_bytes_capacity as usize;
                    let copy_len = byte_len.min(cap);
                    ptr::copy_nonoverlapping(
                        chunk.as_ptr() as *const u8,
                        (*buf).audio_data as *mut u8,
                        copy_len,
                    );
                    (*buf).audio_data_byte_size = copy_len as u32;
                    let enq = AudioQueueEnqueueBuffer(self.queue, buf, 0, ptr::null());
                    if enq != 0 {
                        pool.free_bufs.lock().unwrap().push_back(buf_addr);
                    }
                }
            }
        }

        pub fn flush(&mut self) {
            unsafe {
                AudioQueueReset(self.queue);
            }
        }
    }

    impl Drop for NativeAudioSink {
        fn drop(&mut self) {
            unsafe {
                AudioQueueStop(self.queue, 1);
                AudioQueueDispose(self.queue, 1);
                drop(Box::from_raw(self.pool_ptr));
            }
        }
    }
}

#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
mod imp {
    pub struct NativeAudioSink;

    impl NativeAudioSink {
        pub fn try_open(_sample_rate: u32, _channels: u16) -> Option<Self> {
            None
        }

        pub fn write_pcm(&mut self, _samples: &[i16]) {}

        pub fn flush(&mut self) {}
    }
}

pub struct AudioSink {
    inner: imp::NativeAudioSink,
}

impl AudioSink {
    pub fn try_open(sample_rate: u32, channels: u16) -> Option<Self> {
        imp::NativeAudioSink::try_open(sample_rate, channels).map(|inner| Self { inner })
    }

    pub fn write_pcm(&mut self, samples: &[i16]) {
        self.inner.write_pcm(samples);
    }

    pub fn flush(&mut self) {
        self.inner.flush();
    }
}
