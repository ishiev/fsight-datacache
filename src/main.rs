use structopt::StructOpt;

use warp::Filter;
use warp_reverse_proxy::extract_request_data_filter;
use log::{info, error};

mod datacache;
use datacache::{DataCache, CacheConfig};

mod proxy;
use proxy::{CacheProxy, ProxyConfig};


#[derive(StructOpt, Debug)]
struct Cli {
    port: u16,
}

#[derive(Debug)]
struct Config {
    settings: config::Config,
    args: Cli
}

impl Config {
    fn new(name: &str) -> Self {
        let args = Cli::from_args();

        let mut settings = config::Config::default();
        settings
            // add in `./<name>.toml`
            .merge(config::File::with_name(name)).unwrap_or_else(
                | e | { error!("Error reading config file: {}", e); std::process::exit(-1) }
            )
            // Add in settings from the environment (with a prefix of APP)
            // Eg.. `CACHE_DEBUG=1 ./target/app` would set the `debug` key
            .merge(config::Environment::with_prefix("CACHE")).unwrap();

        Config { settings, args }
    }

    fn get_port(&self) -> u16 { self.args.port }
}

impl CacheConfig for Config {
    fn get_db_path(&self) -> String {
        self.settings
            .get_str("cache.db_path")
            .unwrap_or_else(|_|String::from("data"))
    }
    fn get_ttl(&self) -> u32 {
        3600
    }
}

impl ProxyConfig for Config {
    fn get_proxy_address(&self) -> String {
        self.settings
            .get_str("proxy.target")
            .unwrap_or_default()
    }    
    fn get_host(&self) -> String {
        self.settings
            .get_str("proxy.source")
            .unwrap_or_default()
    }
    fn get_base_path(&self) -> String {
        self.settings
            .get_str("proxy.base_path")
            .unwrap_or_default()
    }
}


#[tokio::main]
async fn main() {
    pretty_env_logger::init_timed();

    info!("starting Forsight BI Server caching proxy");

    // setup settings
    let config = Config::new("settings.toml");

    let cache = DataCache::new(&config);
    let proxy = Box::new(CacheProxy::new(cache, &config));
    let request_filter = extract_request_data_filter();

    //let proxy_map = { move || CacheProxy::new(DataCache::new(&config), &config) };
    let app = warp::get()
        .map(move || { proxy.clone() } )
        .and(request_filter)
        .and_then(CacheProxy::handler);

    warp::serve(app).run(([0, 0, 0, 0], config.get_port())).await;
}
