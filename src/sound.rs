use rodio::Source;
use std::io::{BufReader, Cursor};

const BELL_SINGLE: &[u8] = include_bytes!("../assets/bell_ding.ogg");
const BELL_DOUBLE: &[u8] = include_bytes!("../assets/bell_ding_double.ogg");

pub fn play_ended() {
    play(BELL_SINGLE);
}

pub fn play_waiting() {
    play(BELL_DOUBLE);
}

fn play(data: &'static [u8]) {
    std::thread::spawn(move || {
        let Ok((_stream, handle)) = rodio::OutputStream::try_default() else {
            return;
        };
        let source = rodio::Decoder::new(BufReader::new(Cursor::new(data)));
        if let Ok(source) = source {
            handle.play_raw(source.convert_samples::<f32>()).ok();
            std::thread::sleep(std::time::Duration::from_secs(4));
        }
    });
}
