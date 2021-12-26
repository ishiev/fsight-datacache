use warp::{
    http::Response,
    http::StatusCode,
    Rejection, 
    hyper::body::Bytes,
    hyper::HeaderMap,
    filters::path::FullPath
};

use warp_reverse_proxy::{
    proxy_to_and_forward_response,
    QueryParameters,
    Method,
};

use log::{info, error};
use std::time::Instant;
use tokio::io::AsyncWriteExt; // for write_all()

use crate::datacache::{DataCache, rq_hash_string};


pub trait ProxyConfig {
    fn get_proxy_address(&self) -> String;
    fn get_host(&self) -> String { String::default() }
    fn get_base_path(&self) -> String { String::default() }
    fn get_rq_save_path(&self) -> Option<String> { None }
    fn get_filter_include(&self) -> Vec<Option<String>> { vec!() }
}

pub struct CacheProxy {
    cache: DataCache,
    proxy_address: String,
    host: String,
    base_path: String,
    rq_save_path: Option<String>,
    filter_include: Vec<Option<String>>
}

impl CacheProxy {
    pub fn new<T: ProxyConfig>(cache: DataCache, config: &T) -> Self {
        CacheProxy {
            cache,
            proxy_address: config.get_proxy_address(),
            host: config.get_host(),
            base_path: config.get_base_path(),
            rq_save_path: config.get_rq_save_path(),
            filter_include: config.get_filter_include()
        }
    }

    /// Apply body filter for first 12 bytes, if defined
    fn filter_body(&self, body: &Bytes) -> bool {
        // active only if filters count > 0
        if self.filter_include.len() > 0 {
            const MAGIC_LEN: usize = 12;
            // check for body len, prevent panics
            if body.len() < MAGIC_LEN {
                return false;
            }
            // read first MAGIC_LEN bytes
            let magic = &body[..MAGIC_LEN];
            let count = self.filter_include
                .iter()
                .filter(|f| {
                    if let Some(pattern) = f {
                        return pattern.as_bytes() == magic;
                    }
                    true
                })
                .count();
            return count > 0;
        }   
        true
    }

    /// Save body to file if rq_save_path is set (debug mode)
    async fn save_body(&self, hash: &str, body: &Bytes) -> std::io::Result<()> {
        if let Some(path) = self.rq_save_path.as_deref() {
            // skip write empty files ;)
            if body.len() == 0 {
                info!(
                    "[{}] body empty, skip saving...",
                    &hash[..6]
                );
                return Ok(())
            }
            // ensure all path to file created
            std::fs::create_dir_all(path)?;

            let mut file = tokio::fs::File::create(format!("{}/{}", path, hash)).await?;
            file.write_all(body).await?;
            info!(
                "[{}] body saved to file!",
                &hash[..6]
            );
        }
        Ok(())
    }

    pub async fn handle_request(
        self: std::sync::Arc<CacheProxy>,
        uri: FullPath,
        params: QueryParameters,
        method: Method,
        mut headers: HeaderMap,
        body: Bytes,
    ) -> Result<Response<Bytes>, Rejection> {
        // calculate hash for request 
        let hash = rq_hash_string(&uri, &body);
        info!(
            "[{}] received new request, {}, body len={}",
            &hash[..6], uri.as_str(), body.len()
        );

        // save request body to file if config present, ignore errors...
        let _ = self.save_body(&hash, &body).await;

        // find saved response body in cache database
        if method == Method::GET || method == Method::POST {
            if let Ok(Some(response)) = self.cache.get(&hash) {
                info!(
                    "[{}] return cached response",
                    &hash[..6]
                );
                return Ok(response)
            } 
        }

        // continue processing with request to destination service
        // insert host header from config
        headers.insert("Host", self.host.parse().unwrap());
        // proxy to destination and return response
        match proxy_to_and_forward_response(
            self.proxy_address.to_owned(),
            self.base_path.to_owned(),
            uri,
            params,
            method,
            headers,
            body
        ).await {
            Ok(res) => {
                // save body to cache only if OK 200
                if res.status() == StatusCode::OK {
                    // test body against filter
                    if !self.filter_body(&res.body()) {
                        info!(
                            "[{}] response blocked by filter, not saved to cache",
                            &hash[..6]
                        );
                        return Ok(res);
                    }

                    let timer = Instant::now();
                    if let Err(e) = self.cache.insert(&hash, &res) {
                        error!(
                            "[{}] error saving response to cache, {}",
                            &hash[..6], e
                        )
                    } else {
                        info!(
                            "[{}] new response saved to cache, elapsed={} ms",
                            &hash[..6], timer.elapsed().as_millis()
                        )
                    }
                } else {
                    info!(
                        "[{}] response status code={}, not saved to cache",
                        &hash[..6], res.status()
                    )
                }
                // return response
                Ok(res)
            }
            Err(err) => Err(err)
        }
    }
}
