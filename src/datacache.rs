use std::error::Error;

use chrono::{DateTime, Utc};
use log::{info};
use serde::{Deserialize, Serialize};

use warp::{
    hyper::body::Bytes,
    filters::path::FullPath
};

pub struct DataCache {
    db: sled::Db, // Cache database
    ttl: i64,     // Data Time-To-Live in seconds
}

pub trait CacheConfig {
    fn get_db_path(&self) -> String;
    fn get_ttl(&self) -> i64 {
        3600
    }
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    body: Vec<u8>,
    ctime: DateTime<Utc>,
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

    pub fn get(&self, hash: &str) -> Result<Option<Bytes>, Box<dyn Error>> {
        if let Some(data) = self.db.get(&hash).unwrap() {
            let entry: CacheEntry = bincode::deserialize(&data).unwrap();
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
                // if empty - return not found
                let size = entry.body.len();
                if size > 0 {
                    Ok(Some(Bytes::from(entry.body)))
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

    pub fn insert(&self, hash: &str, body: &Bytes) -> Result<(), Box<dyn Error>> {
        let entry = CacheEntry {
            body: body.to_vec(),
            ctime: Utc::now()
        };
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