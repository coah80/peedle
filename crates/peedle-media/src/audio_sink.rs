#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;
    use std::thread;
    use std::time::Duration;

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

#[cfg(not(target_os = "windows"))]
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
