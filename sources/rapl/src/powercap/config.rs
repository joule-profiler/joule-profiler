use std::{collections::HashSet, time::Duration};

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct RaplConfig {
    #[serde(with = "humantime_serde")]
    pub poll_interval: Option<Duration>,
    pub rapl_path: Option<String>,
    pub sockets_spec: Option<HashSet<u32>>,
}
