use crate::{Error, Result, domain::*};
use reqwest::{Client, Method, StatusCode, Url, header};
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};

#[derive(Clone)]
pub(crate) struct Store {
    client: Client,
    base: Url,
    collection: String,
    pub(crate) profile: Profile,
}
impl Store {
    pub(crate) fn new(
        base: &str,
        collection: String,
        key: Option<String>,
        profile: Profile,
    ) -> Result<Self> {
        let base = Url::parse(base).map_err(|_| Error::Input("invalid Qdrant origin"))?;
        if !matches!(base.scheme(), "http" | "https")
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
        {
            return Err(Error::Input(
                "Qdrant URL must be an origin without credentials",
            ));
        }
        if !collection.starts_with("ytsearch_semantic_pilot_")
            || collection.len() > 128
            || !collection
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(Error::Input(
                "collection must start with ytsearch_semantic_pilot_ and contain only letters, digits, underscores",
            ));
        }
        let mut headers = header::HeaderMap::new();
        if let Some(key) = key {
            let mut key = header::HeaderValue::from_str(&key)
                .map_err(|_| Error::Input("invalid Qdrant key"))?;
            key.set_sensitive(true);
            headers.insert("api-key", key);
        }
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::Store)?;
        Ok(Self {
            client,
            base,
            collection,
            profile,
        })
    }
    pub(crate) fn writer_lock(&self) -> Result<std::fs::File> {
        let identity = digest(format!("{}:{}", self.base, self.collection).as_bytes());
        let path = std::env::temp_dir().join(format!("ytsearch-semantic-{identity}.lock"));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.try_lock()
            .map_err(|_| Error::Input("another local importer owns this collection"))?;
        Ok(file)
    }
    async fn response(
        &self,
        method: Method,
        suffix: &str,
        body: Option<Value>,
    ) -> Result<reqwest::Response> {
        let url = self
            .base
            .join(&format!("collections/{}{}", self.collection, suffix))
            .map_err(|_| Error::Store)?;
        let mut request = self.client.request(method, url);
        if let Some(body) = body {
            request = request.json(&body);
        }
        // Never propagate reqwest errors: they contain URLs. Never log Qdrant error bodies.
        request.send().await.map_err(|_| Error::Store)
    }
    async fn json(&self, method: Method, suffix: &str, body: Option<Value>) -> Result<Value> {
        let mut response = self.response(method, suffix, body).await?;
        if !response.status().is_success() {
            return Err(Error::Store);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Store)? {
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err(Error::Store);
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| Error::Store)?;
        if value["status"] != "ok" {
            return Err(Error::Store);
        }
        Ok(value["result"].clone())
    }
    pub(crate) fn create_body(&self) -> Value {
        json!({"vectors":{"size":self.profile.dimensions.size(),"distance":"Cosine","datatype":"float16","on_disk":true},
            "on_disk_payload":true, "metadata":{"ytsearch_semantic_profile":self.profile.manifest()}})
    }
    fn validate_config(&self, result: &Value) -> Result<()> {
        let config = &result["config"];
        let vectors = &config["params"]["vectors"];
        if vectors["size"] != self.profile.dimensions.size()
            || vectors["distance"] != "Cosine"
            || vectors["datatype"] != "float16"
            || vectors["on_disk"] != true
            || config["params"]["on_disk_payload"] != true
            || config["metadata"]["ytsearch_semantic_profile"] != self.profile.manifest()
        {
            return Err(Error::Profile);
        }
        Ok(())
    }
    pub(crate) async fn ready(&self) -> Result<()> {
        let result = self.json(Method::GET, "", None).await?;
        self.validate_config(&result)
    }
    pub(crate) async fn ensure(&self) -> Result<()> {
        let response = self.response(Method::GET, "", None).await?;
        match response.status() {
            StatusCode::NOT_FOUND => {
                self.json(Method::PUT, "", Some(self.create_body())).await?;
            }
            status if status.is_success() => (),
            _ => return Err(Error::Store),
        }
        self.ready().await?;
        for field in ["profile_id", "video_id", "youtuber_handle"] {
            self.json(
                Method::PUT,
                "/index?wait=true",
                Some(json!({"field_name":field,"field_schema":"keyword"})),
            )
            .await?;
        }
        Ok(())
    }
    pub(crate) async fn existing(&self, video: &str) -> Result<HashMap<String, Value>> {
        let filter = SearchFilter {
            creator: None,
            video_id: Some(video.into()),
        }
        .qdrant(&self.profile);
        let mut output = HashMap::new();
        let mut offset = Value::Null;
        let mut pages = 0;
        loop {
            pages += 1;
            if pages > MAX_VIDEO_CHUNKS * 4 / 128 + 1 {
                return Err(Error::Store);
            }
            let mut body =
                json!({"filter":filter,"limit":128,"with_payload":true,"with_vector":false});
            if !offset.is_null() {
                body["offset"] = offset;
            }
            let page = self
                .json(Method::POST, "/points/scroll", Some(body))
                .await?;
            let points = page["points"].as_array().ok_or(Error::Store)?;
            for point in points {
                let id = point["id"].as_str().ok_or(Error::Store)?;
                if output
                    .insert(id.to_owned(), point["payload"].clone())
                    .is_some()
                    || output.len() > MAX_VIDEO_CHUNKS * 4
                {
                    return Err(Error::Input(
                        "too many existing passages; operator inspection required",
                    ));
                }
            }
            offset = page["next_page_offset"].clone();
            if offset.is_null() {
                break;
            }
        }
        Ok(output)
    }
    pub(crate) async fn upsert(&self, passage: &Passage, vector: Vec<f32>) -> Result<()> {
        if vector.len() != self.profile.dimensions.size() || vector.iter().any(|x| !x.is_finite()) {
            return Err(Error::Model);
        }
        let result = self
            .json(
                Method::PUT,
                "/points?wait=true",
                Some(json!({"points":[{"id":passage.id,"vector":vector,"payload":passage}]})),
            )
            .await?;
        if result["status"] != "completed" {
            return Err(Error::Store);
        }
        Ok(())
    }
    // Same embedded content, newer display metadata (views, likes, names): replace
    // the payload in place and keep the stored vector.
    pub(crate) async fn overwrite_payload(&self, passage: &Passage) -> Result<()> {
        let result = self
            .json(
                Method::PUT,
                "/points/payload?wait=true",
                Some(json!({"payload":passage,"points":[passage.id]})),
            )
            .await?;
        if result["status"] != "completed" {
            return Err(Error::Store);
        }
        Ok(())
    }
    pub(crate) async fn remove_obsolete(&self, ids: &[String]) -> Result<()> {
        // IDs were captured before writing replacements. Delete by ID, not "not in snapshot" filters.
        for batch in ids.chunks(128) {
            let result = self
                .json(
                    Method::POST,
                    "/points/delete?wait=true",
                    Some(json!({"points":batch})),
                )
                .await?;
            if result["status"] != "completed" {
                return Err(Error::Store);
            }
        }
        Ok(())
    }
    pub(crate) async fn search(
        &self,
        vector: Vec<f32>,
        filter: &SearchFilter,
        limit: usize,
    ) -> Result<Vec<Value>> {
        // Collection config is validated at startup and by /ready, not per query;
        // the per-hit profile_id check below still rejects a swapped collection.
        if vector.len() != self.profile.dimensions.size() || vector.iter().any(|x| !x.is_finite()) {
            return Err(Error::Model);
        }
        let response = self.json(Method::POST,"/points/query",Some(json!({"query":vector,
            "filter":filter.qdrant(&self.profile),"limit":limit,"with_payload":true,"with_vector":false}))).await?;
        let points = response["points"].as_array().ok_or(Error::Store)?;
        let mut hits = Vec::new();
        for point in points {
            let payload = &point["payload"];
            if payload["profile_id"] != self.profile.id()
                || !point["score"].as_f64().is_some_and(f64::is_finite)
            {
                return Err(Error::Profile);
            }
            // Hydrated documents have original text/times. No generated/highlighted claims.
            hits.push(json!({"document":payload,"score":point["score"]}));
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collection_config_must_match_the_entire_profile() {
        let store = Store::new(
            "http://127.0.0.1:16333",
            "ytsearch_semantic_pilot_test".into(),
            None,
            Profile {
                dimensions: EmbeddingDimensions::default(),
            },
        )
        .unwrap();
        let body = store.create_body();
        let mut result = json!({"config":{"params":{"vectors":body["vectors"],"on_disk_payload":true},"metadata":body["metadata"]}});
        store.validate_config(&result).unwrap();
        result["config"]["params"]["vectors"]["on_disk"] = json!(false);
        assert!(matches!(
            store.validate_config(&result),
            Err(Error::Profile)
        ));
        result["config"]["params"]["vectors"]["on_disk"] = json!(true);
        result["config"]["params"]["vectors"]["datatype"] = json!("float32");
        assert!(matches!(
            store.validate_config(&result),
            Err(Error::Profile)
        ));
        result["config"]["params"]["vectors"]["datatype"] = json!("float16");
        result["config"]["metadata"]["ytsearch_semantic_profile"]["max_tokens"] = json!(1024);
        assert!(matches!(
            store.validate_config(&result),
            Err(Error::Profile)
        ));
        assert!(
            Store::new(
                "http://user:password@localhost:6333",
                "ytsearch_semantic_pilot_test".into(),
                None,
                store.profile.clone()
            )
            .is_err()
        );
    }
}
