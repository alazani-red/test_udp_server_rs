use serde::{Deserialize, Serialize}; 
use confy;

#[derive(Debug, Deserialize, Serialize, Default)] // Debug, Deserialize, Serialize, Default を derive
pub struct ServerConfig {
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub udp: UdpConfig,
    #[serde(default)]
    pub mq: MqConfig,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct LogConfig {
    pub file: String,      
    pub level: String,
    pub file_num: u32,
    pub file_size: u64,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct UdpConfig {
    pub m_size: usize,
    pub host: Option<String>,
    pub port: u16,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct MqConfig {
    pub client_id: String,
    pub user: String,
    pub passwd: String,
    pub host: String,
    pub port: u16,
    pub keepalive: u64,
    pub qos: u8,
    pub retain: bool,
    pub topic_d: String,
}

pub fn load_config() -> Result<ServerConfig, confy::ConfyError> {
    let config: ServerConfig = confy::load_path("./config.ini")?; // "./" を追加してルートディレクトリを指定
    Ok(config)
}

