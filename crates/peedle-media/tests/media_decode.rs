use peedle_media::{parse_mp4_video_track, H264VideoDecoder, MediaPlayer, StreamingAudioDecoder};
use peedle_net::{HttpClient, HttpMethod, HttpRequest};
use peedle_url::Url;

fn fetch_youtube_stream_urls(client: &HttpClient, video_id: &str) -> (Option<String>, Option<String>, Option<String>) {
    let body = format!(
        "{{\"context\":{{\"client\":{{\"clientName\":\"ANDROID\",\"clientVersion\":\"20.10.38\",\"androidSdkVersion\":34,\"hl\":\"en\",\"gl\":\"US\"}}}},\"videoId\":\"{}\",\"contentCheckOk\":true,\"racyCheckOk\":true}}",
        video_id
    );
    let req = HttpRequest {
        method: HttpMethod::Post,
        url: Url::parse("https://www.youtube.com/youtubei/v1/player?prettyPrint=false")
            .expect("url"),
        headers: vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            (
                "User-Agent".to_string(),
                "com.google.android.youtube/20.10.38 (Linux; U; Android 14) gzip".to_string(),
            ),
        ],
        body: Some(body.into_bytes()),
        max_redirects: 5,
    };
    let resp = client.send(req).expect("innertube request");
    let v: serde_json::Value = serde_json::from_slice(&resp.body).expect("innertube json");
    let mux = v["streamingData"]["formats"]
        .as_array()
        .and_then(|arr| arr.iter().find(|f| f["url"].is_string()))
        .and_then(|f| f["url"].as_str().map(|s| s.to_string()));
    let h264 = v["streamingData"]["adaptiveFormats"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .filter(|f| {
                    f["mimeType"]
                        .as_str()
                        .map_or(false, |m| m.contains("video/mp4") && m.contains("avc1"))
                        && f["url"].is_string()
                })
                .find(|f| f["height"].as_u64().unwrap_or(0) <= 360)
                .or_else(|| {
                    arr.iter().find(|f| {
                        f["mimeType"]
                            .as_str()
                            .map_or(false, |m| m.contains("video/mp4") && m.contains("avc1"))
                            && f["url"].is_string()
                    })
                })
        })
        .and_then(|f| f["url"].as_str().map(|s| s.to_string()));
    let aac = v["streamingData"]["adaptiveFormats"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .find(|f| {
                    f["mimeType"]
                        .as_str()
                        .map_or(false, |m| m.contains("audio/mp4") && m.contains("mp4a.40.2"))
                        && f["url"].is_string()
                })
                .or_else(|| {
                    arr.iter().find(|f| {
                        f["mimeType"]
                            .as_str()
                            .map_or(false, |m| m.contains("audio/webm") && m.contains("opus"))
                            && f["url"].is_string()
                    })
                })
        })
        .and_then(|f| f["url"].as_str().map(|s| s.to_string()));
    (mux, h264, aac)
}

#[test]
fn decode_real_youtube_h264_and_aac() {
    let client = HttpClient::new();
    let (mux_url, dash_v_url, dash_a_url) = fetch_youtube_stream_urls(&client, "jNQXAC9IVRw");
    let mux = mux_url.expect("muxed youtube mp4 url");
    let resp = client.get(&mux).expect("download youtube mp4");
    assert!(resp.status < 400 && resp.body.len() > 50_000);

    let track = parse_mp4_video_track(&resp.body).expect("parse mp4 video track");
    assert!(track.width >= 160 && track.height >= 120);
    assert!(track.samples.len() >= 100);
    assert!(track.duration_sec > 10.0);

    let mut dec = H264VideoDecoder::new();
    let mut decoded_frames = 0usize;
    let mut sample_frame = None;
    for s in track.samples.iter().take(45) {
        if let Some(frame) =
            dec.decode_mp4_sample(&resp.body, s, track.nal_len_size, &track.sps_pps_annex_b)
        {
            decoded_frames += 1;
            if decoded_frames == 30 {
                sample_frame = Some(frame);
            }
        }
    }
    assert!(decoded_frames >= 25, "decoded only {} frames", decoded_frames);
    let f = sample_frame.expect("frame 30");
    let mut sum: u64 = 0;
    let mut min_v = 255u8;
    let mut max_v = 0u8;
    for px in f.rgba.chunks_exact(4) {
        let lum = ((px[0] as u32 + px[1] as u32 + px[2] as u32) / 3) as u8;
        sum += lum as u64;
        min_v = min_v.min(lum);
        max_v = max_v.max(lum);
    }
    let avg = sum as f64 / ((f.width * f.height) as f64);
    assert!(max_v - min_v > 60 && avg > 10.0, "blank frame: avg={} min={} max={}", avg, min_v, max_v);

    let img: image::ImageBuffer<image::Rgba<u8>, _> =
        image::ImageBuffer::from_raw(f.width, f.height, f.rgba).expect("image buf");
    let _ = img.save("/tmp/yt_decoded_frame.png");

    let mut adec = StreamingAudioDecoder::open(resp.body.clone(), Some("mp4")).expect("open audio");
    let mut pcm = Vec::new();
    for _ in 0..40 {
        if !adec
            .decode_next_packet_stereo_i16(1.0, false, &mut pcm)
            .expect("decode audio")
        {
            break;
        }
    }
    assert!(pcm.len() > 10_000, "too few pcm samples: {}", pcm.len());
    let energy: f64 = pcm.iter().map(|&s| (s as f64 / 32768.0).powi(2)).sum::<f64>() / (pcm.len() as f64);
    let rms = energy.sqrt();
    assert!(rms > 0.005, "silent audio rms={}", rms);

    if let (Some(dv), Some(da)) = (dash_v_url, dash_a_url) {
        let player = MediaPlayer::open(client.clone(), dv, Some(da), true);
        let mut got_frame = false;
        for _ in 0..80 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let snap = player.snapshot();
            if !snap.loading && player.current_frame().is_some() {
                assert!(snap.duration > 10.0);
                assert!(snap.has_audio);
                assert!(snap.video_width >= 160);
                got_frame = true;
                break;
            }
        }
        assert!(got_frame, "MediaPlayer failed to decode DASH video+audio");
    }
}
