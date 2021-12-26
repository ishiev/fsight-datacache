use std::error::Error;

use chrono::{DateTime, Utc};
use log::{info};
use serde::{Deserialize, Serialize};

use warp::{
    http::Response,
    http::HeaderMap,
    http::StatusCode,
    http::Error as HttpError,
    hyper::body::Bytes,
    filters::path::FullPath
};


/// Cache configuration
pub trait CacheConfig {
    fn get_db_path(&self) -> String;
    fn get_ttl(&self) -> i64 { 3600 }  // default 1 hour
}

/// Cache entry -- saved response data
#[derive(Serialize, Deserialize)]
struct CacheEntry {
    // response status code
    #[serde(with = "http_serde::status_code")]
    status: StatusCode,

    // response headers (for Access-Control-* and Content-Type)
    #[serde(with = "http_serde::header_map")]
    headers: HeaderMap,

    // saved body
    #[serde(with = "serde_bytes")]
    body: Vec<u8>,

    // date and time entry creation
    ctime: DateTime<Utc>,
}

/// Keys for add header values to cached response, if any
const CACHE_HEADER_KEYS: [&'static str; 9] = [
    "Access-Control-Allow-Headers",
    "Access-Control-Allow-Origin",
    "Access-Control-Allow-Methods",
    "Content-Type",
    "Strict-Transport-Security",
    "Content-Security-Policy",
    "Referrer-Policy",
    "X-XSS-Protection",
    "X-Content-Type-Options"
    ];

const SERVER_NAME: &str = env!("CARGO_PKG_NAME");
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

impl CacheEntry {
    fn to_response(self) -> Result<Response<Bytes>, HttpError> {
        let mut builder = Response::builder()
            .status(self.status)
            .header("Server", format!("{}/{}", SERVER_NAME, SERVER_VERSION));
        
        for key in CACHE_HEADER_KEYS.into_iter() {
            if let Some(value) = self.headers.get(key) {
                builder = builder.header(key, value)
            }
        }
        builder.body(Bytes::from(self.body))
    }

    fn from_response(response: &Response<Bytes>) -> Self {
        CacheEntry {
            status: response.status(),
            headers: response.headers().to_owned(),
            body: response.body().to_vec(),
            ctime: Utc::now()
        }
    }
}

/// Cache for responses data
pub struct DataCache {
    db: sled::Db, // Cache database
    ttl: i64,     // Data Time-To-Live in seconds
}

impl DataCache {
    pub fn new<T: CacheConfig>(config: &T) -> Self {
        // sled db configuration
        let db_config = sled::Config::default()
            .path(config.get_db_path())
            .cache_capacity(10_000_000_000)
            .mode(sled::Mode::HighThroughput)
            .use_compression(true)
            .flush_every_ms(Some(1000));

        let db: sled::Db = db_config.open()
            .expect(format!("error opening cache database: {}", config.get_db_path()).as_str());

        DataCache {
            db: db,
            ttl: config.get_ttl(),
        }
    }

    pub fn get(&self, hash: &str) -> Result<Option<Response<Bytes>>, Box<dyn Error>> {
        if let Some(data) = self.db.get(&hash).unwrap() {
            let entry: CacheEntry = bincode::deserialize(&data)?;
            // test entry ttl
            let age = (Utc::now() - entry.ctime).num_seconds();
            info!(
                "[{}] found result in cache database, age={}",
                &hash[..6],
                age
            );
            if age > self.ttl {
                // entry too old
                info!(
                    "[{}] sorry, result too old, config ttl={}, skipping...",
                    &hash[..6],
                    self.ttl
                );
                Ok(None)
            } else {
                // check size of body
                // if empty - return None
                let size = entry.body.len();
                if size > 0 {
                    // Build response
                    Ok(Some(entry.to_response()?))
                } else {
                    info!(
                        "[{}] sorry, result size={}, skipping...",
                        &hash[..6],
                        size
                    );
                    Ok(None)
                }
            }
        } else {
            // not found
            Ok(None)
        }
    }

    pub fn insert(&self, hash: &str, response: &Response<Bytes>) -> Result<(), Box<dyn Error>> {
        let entry = CacheEntry::from_response(response);
        match self.db.insert(hash, bincode::serialize(&entry)?) {
            Ok(_) => Ok(()),
            Err(err) => Err(Box::new(err))
        } 
    }
}

/// Generate hash string for request uri and body
pub fn rq_hash_string(uri: &FullPath, body: &Bytes) -> String {
    let mut hasher = blake3::Hasher::new();
    // hash uri
    hasher.update(uri.as_str().as_bytes());
    // hash request body
    hasher.update(body);
    // return hash string
    hasher.finalize().to_string()
}