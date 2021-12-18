use std::error::Error;

use chrono::{DateTime, Utc};
use log::{info, debug};
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
        let db: sled::Db = sled::open(config.get_db_path())
            .expect(format!("error opening cache datadase: {}", config.get_db_path()).as_str());
        DataCache {
            db: db,
            ttl: config.get_ttl(),
        }
    }

    pub fn get(&self, hash: &str) -> Result<Option<Bytes>, Box<dyn Error>> {
        if let Some(data) = self.db.get(&hash).unwrap() {
            let entry: CacheEntry = bincode::deserialize(&data).unwrap();
            info!(
                "[{}] found result in cache database, stored at {}",
                &hash[..6],
                entry.ctime
            );
            // test entry ttl
            let ttl = (Utc::now() - entry.ctime).num_seconds();
            debug!(
                "[{}] result ttl={}, config ttl={}",
                &hash[..6],
                ttl, self.ttl
            );
            if ttl > self.ttl {
                // entry too old
                info!(
                    "[{}] sorry, result too old, ttl={}, skipping",
                    &hash[..6],
                    ttl
                );
                Ok(None)
            } else {
                Ok(Some(Bytes::from(entry.body)))
            }
        } else {
            // not found
            Ok(None)
        }
    }

    pub async fn insert(&self, hash: &str, body: &Bytes) -> Result<usize, Box<dyn Error>> {
        let entry = CacheEntry {
            body: body.to_vec(),
            ctime: Utc::now()
        };
        match self.db.insert(hash, bincode::serialize(&entry)?) {
            Ok(_) => {
                // flushes saved data to disk
                match self.db.flush_async().await {
                    Ok(bytes) => Ok(bytes),
                    Err(err) => Err(Box::new(err))
                }
            }
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