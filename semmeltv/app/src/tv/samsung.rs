use super::{Failure, Pairing};
use base64::Engine;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{self, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned};
use crate::db::Setting;
use serde_json::Value;
use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;
use tungstenite::Message;

const NAME: &str = "SemmelTV";
const PORT: u16 = 8002;
const CONNECT_TIME: Duration = Duration::from_secs(3);
const ANSWER_TIME: Duration = Duration::from_secs(40);

pub struct Remote {
    host: String,
    token: Setting,
}

impl Remote {
    pub fn new(host: &str, token: Setting) -> Self {
        Self {
            host: host.to_string(),
            token,
        }
    }

    fn url(&self, token: Option<&str>) -> String {
        let name = base64::engine::general_purpose::STANDARD.encode(NAME);
        let mut url = format!(
            "wss://{}:{PORT}/api/v2/channels/samsung.remote.control?name={}",
            self.host,
            encode(&name)
        );
        if let Some(token) = token {
            url.push_str(&format!("&token={}", encode(token)));
        }
        url
    }

    fn connect(&self, token: Option<&str>) -> Result<Option<String>, Failure> {
        let unreachable = |error: &dyn std::fmt::Display| Failure::Unreachable(error.to_string());
        let address = (self.host.as_str(), PORT)
            .to_socket_addrs()
            .map_err(|error| unreachable(&error))?
            .next()
            .ok_or_else(|| Failure::Unreachable(format!("no address for {}", self.host)))?;
        let tcp = TcpStream::connect_timeout(&address, CONNECT_TIME).map_err(|error| unreachable(&error))?;
        tcp.set_read_timeout(Some(ANSWER_TIME))
            .map_err(|error| unreachable(&error))?;
        tcp.set_write_timeout(Some(CONNECT_TIME))
            .map_err(|error| unreachable(&error))?;
        let provider = Arc::new(crypto::ring::default_provider());
        let config = ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|error| unreachable(&error))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
            .with_no_client_auth();
        let name = ServerName::try_from(self.host.clone()).map_err(|error| unreachable(&error))?;
        let tls = ClientConnection::new(Arc::new(config), name).map_err(|error| unreachable(&error))?;
        let (mut socket, _) =
            tungstenite::client(self.url(token), StreamOwned::new(tls, tcp)).map_err(|error| unreachable(&error))?;
        let answer = loop {
            match socket.read() {
                Ok(Message::Text(text)) => {
                    if let Some(answer) = answer(&text) {
                        break answer;
                    }
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(error))
                    if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) =>
                {
                    break Err(Failure::NoAnswer);
                }
                Err(error) => break Err(unreachable(&error)),
            }
        };
        let _ = socket.close(None);
        answer
    }
}

impl Pairing for Remote {
    fn pair(&self) -> Result<(), Failure> {
        let saved = self.token.get();
        match self.connect(saved.as_deref())? {
            Some(token) if saved.as_ref() != Some(&token) => {
                self.token.set(&token).map_err(|error| Failure::Unsaved(error.to_string()))
            }
            _ => Ok(()),
        }
    }
}

fn answer(text: &str) -> Option<Result<Option<String>, Failure>> {
    let message: Value = serde_json::from_str(text).ok()?;
    let event = message["event"].as_str()?;
    eprintln!("TV pairing: {event}");
    match event {
        "ms.channel.connect" => Some(Ok(message["data"]["token"].as_str().map(str::to_string))),
        "ms.channel.unauthorized" => Some(Err(Failure::Denied)),
        "ms.channel.timeOut" => Some(Err(Failure::NoAnswer)),
        _ => None,
    }
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[derive(Debug)]
struct AnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        crypto::verify_tls12_signature(
            message,
            certificate,
            signature,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        crypto::verify_tls13_signature(
            message,
            certificate,
            signature,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_url_names_semmeltv_and_carries_the_saved_token() {
        let db = crate::db::Db::memory(std::path::Path::new("/lib"));
        let remote = Remote::new("192.168.1.148", db.tv_token());
        assert_eq!(
            remote.url(None),
            "wss://192.168.1.148:8002/api/v2/channels/samsung.remote.control?name=U2VtbWVsVFY%3D"
        );
        assert!(
            remote
                .url(Some("12345678"))
                .ends_with("name=U2VtbWVsVFY%3D&token=12345678")
        );
    }

    #[test]
    fn the_tv_answers_with_a_token_a_refusal_or_a_timeout() {
        let connect = r#"{"event":"ms.channel.connect","data":{"id":"x","token":"12345678"}}"#;
        assert_eq!(answer(connect), Some(Ok(Some("12345678".to_string()))));
        assert_eq!(
            answer(r#"{"event":"ms.channel.connect","data":{"id":"x"}}"#),
            Some(Ok(None))
        );
        assert_eq!(
            answer(r#"{"event":"ms.channel.unauthorized"}"#),
            Some(Err(Failure::Denied))
        );
        assert_eq!(
            answer(r#"{"event":"ms.channel.timeOut"}"#),
            Some(Err(Failure::NoAnswer))
        );
        assert_eq!(answer(r#"{"event":"ms.channel.clientConnect"}"#), None);
        assert_eq!(answer("not json"), None);
    }
}
