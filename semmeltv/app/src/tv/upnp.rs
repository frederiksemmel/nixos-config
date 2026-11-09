use super::{Failure, Volume};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SERVICE: &str = "urn:schemas-upnp-org:service:RenderingControl:1";
const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug)]
struct Description {
    control: String,
    manufacturer: String,
}

#[derive(Clone)]
pub struct Renderer {
    description: String,
    agent: ureq::Agent,
    read: Arc<Mutex<Option<Description>>>,
}

impl Renderer {
    pub fn new(description: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .build()
            .into();
        Self {
            description,
            agent,
            read: Arc::new(Mutex::new(None)),
        }
    }

    pub fn host(&self) -> &str {
        let after_scheme = self.description.find("://").map_or(0, |at| at + 3);
        let rest = &self.description[after_scheme..];
        let end = rest.find([':', '/']).unwrap_or(rest.len());
        &rest[..end]
    }

    fn read(&self) -> Result<Description, Failure> {
        if let Some(description) = self.read.lock().unwrap().clone() {
            return Ok(description);
        }
        let text = self
            .agent
            .get(&self.description)
            .call()
            .and_then(|mut response| response.body_mut().read_to_string())
            .map_err(|error| Failure::Unreachable(error.to_string()))?;
        let path = control_path(&text).ok_or_else(|| Failure::Garbled("no RenderingControl service".to_string()))?;
        let description = Description {
            control: resolve(&self.description, path),
            manufacturer: element(&text, "manufacturer").unwrap_or_default().trim().to_string(),
        };
        *self.read.lock().unwrap() = Some(description.clone());
        Ok(description)
    }

    pub fn manufacturer(&self) -> Result<String, Failure> {
        self.read().map(|description| description.manufacturer)
    }

    fn call(&self, action: &str, arguments: &str) -> Result<String, Failure> {
        let url = self.read()?.control;
        let body = envelope(action, arguments);
        let (status, text) = self
            .agent
            .post(&url)
            .header("SOAPAction", format!("\"{SERVICE}#{action}\""))
            .header("Content-Type", "text/xml; charset=\"utf-8\"")
            .header("Connection", "close")
            .send(body.as_bytes())
            .and_then(|mut response| {
                let status = response.status().as_u16();
                Ok((status, response.body_mut().read_to_string()?))
            })
            .map_err(|error| Failure::Unreachable(error.to_string()))?;
        match (element(&text, "errorCode"), status) {
            (Some(code), _) => {
                let description = element(&text, "errorDescription").unwrap_or("no reason given");
                Err(Failure::Refused(format!("{code} {description}")))
            }
            (None, 200..=299) => Ok(text),
            (None, status) => Err(Failure::Refused(format!("HTTP {status}"))),
        }
    }

    fn number(&self, action: &str, name: &str) -> Result<i64, Failure> {
        let text = self.call(action, "<Channel>Master</Channel>")?;
        element(&text, name)
            .and_then(|value| value.trim().parse().ok())
            .ok_or_else(|| Failure::Garbled(format!("no {name}")))
    }
}

impl Volume for Renderer {
    fn step(&self, by: i64) -> Result<i64, Failure> {
        let volume = (self.number("GetVolume", "CurrentVolume")? + by).clamp(0, 100);
        self.call(
            "SetVolume",
            &format!("<Channel>Master</Channel><DesiredVolume>{volume}</DesiredVolume>"),
        )?;
        Ok(volume)
    }

    fn toggle_mute(&self) -> Result<bool, Failure> {
        let muted = self.number("GetMute", "CurrentMute")? == 0;
        self.call(
            "SetMute",
            &format!(
                "<Channel>Master</Channel><DesiredMute>{}</DesiredMute>",
                u8::from(muted)
            ),
        )?;
        Ok(muted)
    }
}

fn envelope(action: &str, arguments: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{SERVICE}\">\
         <InstanceID>0</InstanceID>{arguments}</u:{action}></s:Body></s:Envelope>"
    )
}

fn element<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = start + xml[start..].find(&format!("</{name}>"))?;
    Some(&xml[start..end])
}

fn control_path(description: &str) -> Option<&str> {
    let service = description.find(SERVICE)?;
    element(&description[service..], "controlURL").map(str::trim)
}

fn resolve(description: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    let after_scheme = description.find("://").map_or(0, |at| at + 3);
    let origin = description[after_scheme..]
        .find('/')
        .map_or(description, |at| &description[..after_scheme + at]);
    format!("{origin}/{}", path.trim_start_matches('/'))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use tokio::net::TcpListener;

    const DESCRIPTION: &str = "<root><device><manufacturer>Samsung Electronics</manufacturer><serviceList>\
        <service><serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>\
        <controlURL>/upnp/control/AVTransport1</controlURL></service>\
        <service><serviceType>urn:schemas-upnp-org:service:RenderingControl:1</serviceType>\
        <controlURL>/upnp/control/RenderingControl1</controlURL></service>\
        </serviceList></device></root>";

    #[derive(Clone, Default)]
    pub struct Tv {
        pub volume: Arc<Mutex<i64>>,
        pub muted: Arc<Mutex<bool>>,
        pub refuse: bool,
        pub received: Arc<Mutex<Vec<String>>>,
    }

    async fn rendering_control(State(tv): State<Tv>, headers: HeaderMap, body: String) -> Response {
        let action = headers["soapaction"]
            .to_str()
            .unwrap()
            .rsplit('#')
            .next()
            .unwrap()
            .trim_matches('"')
            .to_string();
        tv.received.lock().unwrap().push(body.clone());
        if tv.refuse && action.starts_with("Set") {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "<s:Envelope><s:Body><s:Fault><detail><UPnPError><errorCode>501</errorCode>\
                 <errorDescription>Action Failed</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>",
            )
                .into_response();
        }
        let reply = match action.as_str() {
            "GetVolume" => format!("<CurrentVolume>{}</CurrentVolume>", tv.volume.lock().unwrap()),
            "GetMute" => format!("<CurrentMute>{}</CurrentMute>", u8::from(*tv.muted.lock().unwrap())),
            "SetVolume" => {
                *tv.volume.lock().unwrap() = element(&body, "DesiredVolume").unwrap().parse().unwrap();
                "<u:SetVolumeResponse/>".to_string()
            }
            "SetMute" => {
                *tv.muted.lock().unwrap() = element(&body, "DesiredMute") == Some("1");
                "<u:SetMuteResponse/>".to_string()
            }
            _ => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        reply.into_response()
    }

    pub struct Fake {
        pub tv: Tv,
        base: String,
        _runtime: tokio::runtime::Runtime,
    }

    impl Fake {
        pub fn renderer(&self) -> Renderer {
            Renderer::new(format!("{}/dmr", self.base))
        }
    }

    pub fn fake(tv: Tv) -> Fake {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/dmr", get(|| async { DESCRIPTION }))
            .route("/upnp/control/RenderingControl1", post(rendering_control))
            .with_state(tv.clone());
        runtime.spawn(async move { axum::serve(listener, router).await });
        Fake {
            tv,
            base,
            _runtime: runtime,
        }
    }

    #[test]
    fn the_description_gives_the_control_url_and_the_manufacturer() {
        assert_eq!(control_path(DESCRIPTION), Some("/upnp/control/RenderingControl1"));
        assert_eq!(
            resolve("http://192.168.1.148:9197/dmr", "/upnp/control/RenderingControl1"),
            "http://192.168.1.148:9197/upnp/control/RenderingControl1"
        );
        assert_eq!(resolve("http://tv:9197/dmr", "control"), "http://tv:9197/control");
        assert_eq!(resolve("http://tv:9197/dmr", "http://other/x"), "http://other/x");
        let fake = fake(Tv::default());
        assert_eq!(fake.renderer().manufacturer(), Ok("Samsung Electronics".to_string()));
    }

    #[test]
    fn the_host_comes_from_the_description_url() {
        assert_eq!(
            Renderer::new("http://192.168.1.148:9197/dmr".to_string()).host(),
            "192.168.1.148"
        );
        assert_eq!(
            Renderer::new("http://tv.lan/description.xml".to_string()).host(),
            "tv.lan"
        );
    }

    #[test]
    fn volume_steps_within_its_range() {
        let tv = Tv::default();
        *tv.volume.lock().unwrap() = 11;
        let fake = fake(tv);
        let renderer = fake.renderer();
        assert_eq!(renderer.step(2), Ok(13));
        assert_eq!(*fake.tv.volume.lock().unwrap(), 13);
        assert_eq!(renderer.step(-2), Ok(11));
        *fake.tv.volume.lock().unwrap() = 99;
        assert_eq!(renderer.step(2), Ok(100));
        *fake.tv.volume.lock().unwrap() = 1;
        assert_eq!(renderer.step(-2), Ok(0));
        let set = fake
            .tv
            .received
            .lock()
            .unwrap()
            .iter()
            .find(|body| body.contains("SetVolume"))
            .cloned()
            .unwrap();
        assert!(set.contains("<InstanceID>0</InstanceID><Channel>Master</Channel><DesiredVolume>13</DesiredVolume>"));
    }

    #[test]
    fn mute_toggles() {
        let fake = fake(Tv::default());
        let renderer = fake.renderer();
        assert_eq!(renderer.toggle_mute(), Ok(true));
        assert!(*fake.tv.muted.lock().unwrap());
        assert_eq!(renderer.toggle_mute(), Ok(false));
    }

    #[test]
    fn a_refusal_is_reported_as_one() {
        let fake = fake(Tv {
            refuse: true,
            ..Tv::default()
        });
        assert_eq!(
            fake.renderer().step(2),
            Err(Failure::Refused("501 Action Failed".to_string()))
        );
    }

    #[test]
    fn an_unreachable_tv_says_so() {
        let failure = Renderer::new("http://127.0.0.1:9/dmr".to_string()).step(2).unwrap_err();
        assert!(matches!(failure, Failure::Unreachable(_)), "{failure:?}");
    }
}
