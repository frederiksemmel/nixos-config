use serde_json::{Value, json};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{Shutdown, TcpStream};
use std::thread;
use std::time::Duration;

pub const RATE: u32 = 16_000;
const WIDTH: u32 = 2;
const CHANNELS: u32 = 1;
const VERSION: &str = "1.9.0";
const CHUNK_SAMPLES: usize = 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, PartialEq)]
pub struct Event {
    pub kind: String,
    pub data: Value,
    pub payload: Vec<u8>,
}

pub struct Connection {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Connection {
    pub fn open(address: &str) -> io::Result<Self> {
        let stream = TcpStream::connect(address.trim_start_matches("tcp://"))?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        Ok(Self {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
        })
    }

    pub fn send(&mut self, kind: &str, data: Value, payload: &[u8]) -> io::Result<()> {
        self.writer.write_all(&encode(kind, data, payload))
    }

    pub fn send_audio(&mut self, samples: &[i16]) -> io::Result<()> {
        self.send("audio-start", format(), &[])?;
        for chunk in samples.chunks(CHUNK_SAMPLES) {
            self.send("audio-chunk", format(), &to_bytes(chunk))?;
        }
        self.send("audio-stop", json!({}), &[])
    }

    pub fn receive(&mut self) -> io::Result<Event> {
        decode(&mut self.reader)
    }
}

pub fn format() -> Value {
    json!({ "rate": RATE, "width": WIDTH, "channels": CHANNELS })
}

pub fn transcribe(address: &str, samples: &[i16]) -> io::Result<String> {
    let mut connection = Connection::open(address)?;
    connection.send("transcribe", json!({}), &[])?;
    connection.send_audio(samples)?;
    loop {
        let event = connection.receive()?;
        if event.kind == "transcript" {
            return Ok(event.data["text"].as_str().unwrap_or_default().trim().to_string());
        }
    }
}

pub struct Detector {
    writer: TcpStream,
}

impl Detector {
    pub fn restart(&mut self) -> io::Result<()> {
        self.writer.write_all(&encode("audio-start", format(), &[]))
    }

    pub fn send(&mut self, samples: &[i16]) -> io::Result<()> {
        self.writer
            .write_all(&encode("audio-chunk", format(), &to_bytes(samples)))
    }
}

impl Drop for Detector {
    fn drop(&mut self) {
        let _ = self.writer.shutdown(Shutdown::Both);
    }
}

pub fn detect(address: &str, name: &str, detected: impl Fn() + Send + 'static) -> io::Result<Detector> {
    let mut connection = Connection::open(address)?;
    connection.reader.get_ref().set_read_timeout(None)?;
    connection.send("detect", json!({ "names": [name] }), &[])?;
    let Connection { mut reader, writer } = connection;
    thread::spawn(move || {
        while let Ok(event) = decode(&mut reader) {
            if event.kind == "detection" {
                detected();
            }
        }
    });
    let mut detector = Detector { writer };
    detector.restart()?;
    Ok(detector)
}

fn to_bytes(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|sample| sample.to_le_bytes()).collect()
}

fn encode(kind: &str, data: Value, payload: &[u8]) -> Vec<u8> {
    let mut header = json!({ "type": kind, "version": VERSION, "data": data });
    if !payload.is_empty() {
        header["payload_length"] = payload.len().into();
    }
    let mut bytes = header.to_string().into_bytes();
    bytes.push(b'\n');
    bytes.extend_from_slice(payload);
    bytes
}

fn decode(reader: &mut impl BufRead) -> io::Result<Event> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    let header: Value = serde_json::from_str(&line)?;
    let mut data = header.get("data").cloned().unwrap_or_else(|| json!({}));
    if let Some(length) = length(&header, "data_length") {
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes)?;
        let extra: Value = serde_json::from_slice(&bytes)?;
        if let (Some(data), Some(extra)) = (data.as_object_mut(), extra.as_object()) {
            data.extend(extra.clone());
        }
    }
    let mut payload = vec![0; length(&header, "payload_length").unwrap_or_default()];
    reader.read_exact(&mut payload)?;
    Ok(Event {
        kind: header["type"].as_str().unwrap_or_default().to_string(),
        data,
        payload,
    })
}

fn length(header: &Value, key: &str) -> Option<usize> {
    header[key].as_u64().and_then(|length| usize::try_from(length).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::net::TcpListener;
    use std::sync::mpsc;

    #[test]
    fn events_survive_a_round_trip() {
        let bytes = encode("audio-chunk", format(), &[1, 2, 3, 4]);
        let event = decode(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(event.kind, "audio-chunk");
        assert_eq!(event.data["rate"], 16_000);
        assert_eq!(event.payload, vec![1, 2, 3, 4]);
    }

    #[test]
    fn data_sent_after_the_header_is_merged() {
        let data = r#"{"text":"Pause."}"#;
        let bytes = format!("{{\"type\":\"transcript\",\"data_length\":{}}}\n{data}", data.len());
        let event = decode(&mut Cursor::new(bytes.into_bytes())).unwrap();
        assert_eq!(event.data["text"], "Pause.");
    }

    #[test]
    fn transcription_streams_the_audio_and_waits_for_the_transcript() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut kinds = Vec::new();
            let mut samples = 0;
            while kinds.last().is_none_or(|kind| kind != "audio-stop") {
                let event = decode(&mut reader).unwrap();
                samples += event.payload.len() / 2;
                kinds.push(event.kind);
            }
            let mut writer = stream;
            writer.write_all(&encode("transcript-start", json!({}), &[])).unwrap();
            writer
                .write_all(&encode("transcript", json!({ "text": " Pause. " }), &[]))
                .unwrap();
            (kinds, samples)
        });
        let text = transcribe(&format!("tcp://{address}"), &[0; 2500]).unwrap();
        let (kinds, samples) = server.join().unwrap();
        assert_eq!(text, "Pause.");
        assert_eq!(samples, 2500);
        assert_eq!(
            kinds,
            [
                "transcribe",
                "audio-start",
                "audio-chunk",
                "audio-chunk",
                "audio-chunk",
                "audio-stop"
            ]
        );
    }

    #[test]
    fn detection_streams_audio_and_reports_the_wake_word() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let detect = decode(&mut reader).unwrap();
            let start = decode(&mut reader).unwrap();
            let chunk = decode(&mut reader).unwrap();
            let mut writer = stream;
            writer
                .write_all(&encode(
                    "detection",
                    json!({ "name": "stevie_the_tv", "timestamp": 1230 }),
                    &[],
                ))
                .unwrap();
            (detect, start.kind, chunk.payload.len())
        });
        let (detections, received) = mpsc::channel();
        let mut detector = detect(&address, "stevie_the_tv", move || {
            let _ = detections.send(());
        })
        .unwrap();
        detector.send(&[0; 480]).unwrap();
        let (detect, start, chunk) = server.join().unwrap();
        assert_eq!(detect.kind, "detect");
        assert_eq!(detect.data["names"], json!(["stevie_the_tv"]));
        assert_eq!(start, "audio-start");
        assert_eq!(chunk, 960);
        assert_eq!(received.recv_timeout(TIMEOUT), Ok(()));
    }
}
