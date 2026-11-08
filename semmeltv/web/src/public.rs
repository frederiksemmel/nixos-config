#[derive(Clone)]
pub struct Public {
    pub api: String,
}

impl Public {
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.api)
    }
}
