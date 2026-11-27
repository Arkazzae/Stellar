//! What one session may send: the limits on the packets of each published
//! track and on the datagrams of a session, with real protocol stacks on both
//! ends.

mod common;

use std::time::Duration;

use common::*;
use stellarr_media::core::{
    DATAGRAMS_PER_SECOND, PICTURE_PACKETS_PER_SECOND, SOUND_BYTES_PER_SECOND,
    SOUND_PACKETS_PER_SECOND, SessionStats, Stats,
};
use str0m::format::Codec;
use str0m::media::{MediaKind, Mid};

/// The mid on which client `i` receives the track of `kind` that its
/// publisher put in stream `stream`.
fn mid_of(net: &Net, i: usize, stream: &str, kind: &str) -> String {
    net.client(i)
        .directory
        .iter()
        .find(|t| t["streamId"] == stream && t["kind"] == kind)
        .and_then(|t| t["mid"].as_str())
        .unwrap_or_else(|| panic!("client {i} receives no {kind} of {stream}"))
        .to_string()
}

/// How many packets of `stream`'s track of `kind` client `i` got.
fn got(net: &Net, i: usize, stream: &str, kind: &str) -> usize {
    net.client(i)
        .received_on(&mid_of(net, i, stream, kind))
        .len()
}

fn send_video(net: &mut Net, i: usize, mid: Mid, payload: &[u8]) {
    let pt = net.client(i).pt_of(Codec::Vp8);
    net.send(i, mid, pt, payload);
}

/// Sends one frame of `bytes` in the packets a browser would split it into,
/// and returns how many it took.
fn send_frame(net: &mut Net, i: usize, mid: Mid, bytes: usize) -> usize {
    let frame = vec![9; bytes];
    for packet in frame.chunks(VIDEO_PACKET) {
        send_video(net, i, mid, packet);
    }
    frame.chunks(VIDEO_PACKET).count()
}

/// The stats of client `i`'s session.
fn session<'a>(net: &Net, stats: &'a Stats, i: usize) -> &'a SessionStats {
    let id = &net.client(i).session_id;
    stats.rooms[0]
        .sessions
        .iter()
        .find(|s| s.session_id == *id)
        .expect("the client's session")
}

/// Runs until every client is connected and receives `tracks` tracks.
fn settle(net: &mut Net, tracks: &[usize]) {
    let tracks = tracks.to_vec();
    net.run_until(Duration::from_secs(5), move |n| {
        n.clients
            .iter()
            .zip(&tracks)
            .all(|(c, &count)| c.is_connected() && c.directory.len() == count)
    });
    net.run_for(Duration::from_millis(100));
}

#[test]
fn what_the_web_client_sends_at_its_most_is_forwarded_whole() {
    let mut net = Net::new();
    let a = net.join(1);
    // The receiver sends no transport-wide feedback, so its downlink is never
    // measured and nothing is paced or held back for it: what arrives is
    // what the limits let through.
    let b = net.join_with(ROOM, 2, |c| c.clear_extension_map());
    let mic = net.publish(a, MediaKind::Audio, "voice");
    let camera = net.publish(a, MediaKind::Video, "voice");
    let screen = net.publish(a, MediaKind::Video, "screen");
    let shared = net.publish(a, MediaKind::Audio, "screen");
    net.publish(b, MediaKind::Audio, "listener");
    settle(&mut net, &[1, 4]);

    // Three seconds of the most the web client sends: the studio microphone
    // (256 kbit/s) in the 10 ms packets `minptime=10` allows and the shared
    // sound (160 kbit/s) in 20 ms ones, both with room for the variable rate
    // and for SFrame; a 3 Mbit/s camera at 30 frames a second and a 12 Mbit/s
    // screen share at 60. Each picture starts on a keyframe, and the share
    // sends another halfway: half a megabyte at once.
    let mut sent = [0; 4];
    for ms in 0..3000 {
        if ms % 10 == 0 {
            net.send_audio(a, mic, &[1; 360]);
            sent[0] += 1;
        }
        if ms % 20 == 0 {
            net.send_audio(a, shared, &[2; 440]);
            sent[1] += 1;
        }
        if ms % 33 == 0 {
            let bytes = if ms == 0 { 125_000 } else { 12_500 };
            sent[2] += send_frame(&mut net, a, camera, bytes);
        }
        if ms % 16 == 0 {
            let keyframe = ms == 0 || ms == 1504;
            let bytes = if keyframe { 500_000 } else { 24_000 };
            sent[3] += send_frame(&mut net, a, screen, bytes);
        }
        net.step(TICK);
    }
    net.run_for(Duration::from_secs(2));

    let tracks = [
        ("voice", "audio"),
        ("screen", "audio"),
        ("voice", "video"),
        ("screen", "video"),
    ];
    for ((stream, kind), sent) in tracks.into_iter().zip(sent) {
        assert_eq!(got(&net, b, stream, kind), sent, "{kind} of {stream}");
    }
    let stats = net.core.stats(net.now);
    for i in [a, b] {
        let dropped = session(&net, &stats, i).dropped;
        assert_eq!((dropped.datagrams, dropped.packets), (0, 0), "client {i}");
    }
}

#[test]
fn a_flood_on_a_track_is_cut_to_its_limit_for_everyone_and_other_voices_arrive_whole() {
    let mut net = Net::new();
    let many = net.join(1);
    let large = net.join(2);
    let pictures = net.join(3);
    let speaker = net.join(4);
    let listener = net.join(5);
    let mic_many = net.publish(many, MediaKind::Audio, "many");
    let mic_large = net.publish(large, MediaKind::Audio, "large");
    let camera = net.publish(pictures, MediaKind::Video, "pictures");
    let mic_speaker = net.publish(speaker, MediaKind::Audio, "speaker");
    net.publish(listener, MediaKind::Audio, "listener");
    settle(&mut net, &[4; 5]);

    // For two seconds, one microphone sends 4,000 small packets a second,
    // another 125 packets of 1,200 bytes, and a camera 4,000 small packets.
    // Each session stays under its limit on datagrams, so only the limits of
    // the tracks are at work. Somebody else speaks as usual.
    const LARGE: usize = 1200;
    let mut spoken = 0;
    for ms in 0..2000 {
        for _ in 0..4 {
            net.send_audio(many, mic_many, &[1; 20]);
            send_video(&mut net, pictures, camera, &[3; 20]);
        }
        if ms % 8 == 0 {
            net.send_audio(large, mic_large, &[2; LARGE]);
        }
        if ms % 20 == 0 {
            net.send_audio(speaker, mic_speaker, &[4; 80]);
            spoken += 1;
        }
        net.step(TICK);
    }
    net.run_for(Duration::from_secs(1));

    // Each flood gets the second its bucket holds and two of its rate, less
    // what the last moments of the flood left unspent, and every other
    // client gets the same of it.
    let stats = net.core.stats(net.now);
    let sound = 3 * SOUND_PACKETS_PER_SECOND;
    let sound_bytes = 3 * SOUND_BYTES_PER_SECOND / LARGE as u64;
    let picture = 3 * PICTURE_PACKETS_PER_SECOND;
    let floods = [
        (many, "many", "audio", 8000, sound),
        (large, "large", "audio", 250, sound_bytes),
        (pictures, "pictures", "video", 8000, picture),
    ];
    for (flooder, stream, kind, sent, limit) in floods {
        let counts: Vec<u64> = (0..5)
            .filter(|&i| i != flooder)
            .map(|i| got(&net, i, stream, kind) as u64)
            .collect();
        assert!(
            counts.iter().all(|&n| n == counts[0]),
            "{stream}: {counts:?}"
        );
        let forwarded = counts[0];
        assert!(
            (limit - 3..=limit).contains(&forwarded),
            "{stream}: {forwarded} of {sent}, at most {limit}"
        );
        let s = session(&net, &stats, flooder);
        assert_eq!(s.ingress.packets, sent, "{stream}");
        assert_eq!(s.dropped.packets, sent - forwarded, "{stream}");
        assert_eq!(s.dropped.datagrams, 0, "{stream}");
    }
    for i in [many, large, pictures, listener] {
        assert_eq!(got(&net, i, "speaker", "audio"), spoken, "client {i}");
    }
    for i in [speaker, listener] {
        let dropped = session(&net, &stats, i).dropped;
        assert_eq!((dropped.datagrams, dropped.packets), (0, 0), "client {i}");
    }
}

/// An RTP packet of an SSRC that no stream has. Its session's connection
/// drops it unread, once the limit has counted it.
fn stray_rtp() -> Vec<u8> {
    let mut packet = vec![0; 40];
    packet[0] = 0x80;
    packet[1] = 96;
    packet[8..12].copy_from_slice(&0x5eed_f00d_u32.to_be_bytes());
    packet
}

#[test]
fn a_session_over_its_datagram_limit_loses_media_and_keeps_its_connection() {
    let mut net = Net::new();
    let flooder = net.join(1);
    let listener = net.join(2);
    let mic = net.publish(flooder, MediaKind::Audio, "flooder");
    net.publish(listener, MediaKind::Audio, "listener");
    settle(&mut net, &[1, 1]);

    // For longer than the 15 s str0m's ICE-lite side keeps a path without a
    // check, the flooder's address sends twice the datagrams its session may,
    // ahead of everything else its client sends in each millisecond. Its
    // microphone speaks as usual all along.
    let source = net.client(flooder).seen_at();
    let stray = stray_rtp();
    let seconds = 18;
    let start = net.now;
    let mut flooded = 0;
    for ms in 0..seconds * 1000 {
        let next = net.now + TICK;
        for _ in 0..2 * DATAGRAMS_PER_SECOND / 1000 {
            net.core.handle_datagram(next, source, &stray);
            flooded += 1;
        }
        net.step(TICK);
        if ms % 20 == 0 {
            net.send_audio(flooder, mic, &[4; 80]);
        }
    }

    // Once the second the bucket holds was spent, nothing of the voice got
    // through, and what was dropped is counted.
    let mid = mid_of(&net, listener, "flooder", "audio");
    let late = start + Duration::from_secs(2);
    let heard_late = net
        .client(listener)
        .received
        .iter()
        .filter(|r| *r.mid == *mid && r.at >= late)
        .count();
    assert_eq!(heard_late, 0);
    let stats = net.core.stats(net.now);
    let s = session(&net, &stats, flooder);
    let allowed = DATAGRAMS_PER_SECOND * (seconds + 1);
    assert!(
        s.dropped.datagrams >= flooded - allowed,
        "{} of {flooded} dropped",
        s.dropped.datagrams
    );
    assert_eq!(s.dropped.packets, 0);

    // The checks got through on their own bucket all along.
    assert_eq!(s.state, "connected");
    assert!(net.client(flooder).closed.is_none());
    assert!(net.client(flooder).failure.is_none());

    // Once the flood stops, the voice is taken again.
    net.run_for(Duration::from_secs(1));
    let before = got(&net, listener, "flooder", "audio");
    net.send_audio(flooder, mic, &[5; 80]);
    net.run_for(Duration::from_millis(100));
    assert_eq!(got(&net, listener, "flooder", "audio"), before + 1);
}
