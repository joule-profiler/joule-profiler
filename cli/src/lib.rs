use log::LevelFilter;

pub mod cli;
pub mod config;
pub mod exporter;
pub mod sources;

pub fn init_logging(verbose: u8) {
    let level = match verbose {
        0 => LevelFilter::Warn,
        1 => LevelFilter::Info,
        2 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    env_logger::Builder::new().filter_level(level).init();
}
