//! E387: a text embedder in the kernel, for BERT-class sentence models
//! such as bge-small-en-v1.5 (E306, E312: the strongest embedder measured
//! for fused search). Off by default: built only with the `embed` feature,
//! and nothing embeds unless a caller opens a model.
//!
//! A model is a folder with `config.json`, `tokenizer.json` and
//! `model.safetensors`, as the model's own release ships them; weights are
//! never bundled or downloaded here. `Embedder::open` reads the folder and,
//! given a pinned BLAKE3 of the weights, refuses any other file. Vectors
//! are stored under `Embedder::id` (`embeds`, E255), a BLAKE3 of the
//! weights, the tokenizer, the config, `max_tokens` and the pooling: a
//! model opened with any of them changed gets another id, so its vectors
//! are never mixed with the old ones. A vector is the first token's last hidden state (the model's
//! CLS pooling), L2-normalised, from at most `max_tokens` tokens; texts in
//! one call are padded to the longest and masked, so a text's vector does
//! not depend on what else is in its batch (to float rounding).

use std::path::Path;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

/// What `bge` models put before a query (not before a passage).
pub const BGE_QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("model folder: {0}")]
    Io(#[from] std::io::Error),
    #[error("model config: {0}")]
    Config(String),
    #[error("tokenizer: {0}")]
    Tokenizer(String),
    #[error("model: {0}")]
    Model(#[from] candle_core::Error),
    #[error("weights are {found}, pinned {pinned}")]
    Pin { found: String, pinned: String },
}

pub struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    /// BLAKE3 of `model.safetensors`, hex (what `open` pins).
    pub weights: String,
    /// The id for stored vectors, hex: a BLAKE3 of everything that decides
    /// a vector (weights, `tokenizer.json`, `config.json`, `max_tokens`,
    /// pooling).
    pub id: String,
    pub dims: usize,
    pub max_tokens: usize,
}

impl Embedder {
    /// Open a model folder, on the CPU. With `pinned`, the weights' BLAKE3
    /// (hex) must equal it.
    pub fn open(dir: &Path, pinned: Option<&str>, max_tokens: usize) -> Result<Self, EmbedError> {
        let bytes = std::fs::read(dir.join("model.safetensors"))?;
        let weights = blake3::hash(&bytes).to_hex().to_string();
        if let Some(p) = pinned.filter(|p| !p.eq_ignore_ascii_case(&weights)) {
            return Err(EmbedError::Pin {
                found: weights,
                pinned: p.into(),
            });
        }
        let cfg = std::fs::read_to_string(dir.join("config.json"))?;
        let config: Config =
            serde_json::from_str(&cfg).map_err(|e| EmbedError::Config(e.to_string()))?;
        let vb = VarBuilder::from_buffered_safetensors(bytes, DType::F32, &Device::Cpu)?;
        let model = BertModel::load(vb, &config)?;
        let tok = std::fs::read(dir.join("tokenizer.json"))?;
        let mut tokenizer =
            Tokenizer::from_bytes(&tok).map_err(|e| EmbedError::Tokenizer(e.to_string()))?;
        let max_tokens = max_tokens.min(config.max_position_embeddings);
        let mut h = blake3::Hasher::new();
        h.update(b"ikam-embed/1 cls-l2\0");
        for part in [weights.as_bytes(), &tok, cfg.as_bytes()] {
            h.update(&(part.len() as u64).to_le_bytes());
            h.update(part);
        }
        h.update(&(max_tokens as u64).to_le_bytes());
        let id = h.finalize().to_hex().to_string();
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: max_tokens,
                ..Default::default()
            }))
            .map_err(|e| EmbedError::Tokenizer(e.to_string()))?;
        tokenizer.with_padding(Some(PaddingParams::default()));
        Ok(Self {
            model,
            tokenizer,
            weights,
            id,
            dims: config.hidden_size,
            max_tokens,
        })
    }

    /// One unit vector a text, in order.
    pub fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let enc = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| EmbedError::Tokenizer(e.to_string()))?;
        let (n, len) = (enc.len(), enc[0].get_ids().len());
        let flat = |f: &dyn Fn(&tokenizers::Encoding) -> &[u32]| -> Vec<u32> {
            enc.iter().flat_map(|e| f(e).iter().copied()).collect()
        };
        let dev = Device::Cpu;
        let ids = Tensor::from_vec(flat(&|e| e.get_ids()), (n, len), &dev)?;
        let types = Tensor::from_vec(flat(&|e| e.get_type_ids()), (n, len), &dev)?;
        let mask = Tensor::from_vec(flat(&|e| e.get_attention_mask()), (n, len), &dev)?;
        let hidden = self.model.forward(&ids, &types, Some(&mask))?;
        let cls = hidden.narrow(1, 0, 1)?.squeeze(1)?;
        let norm = cls.sqr()?.sum_keepdim(1)?.sqrt()?;
        Ok(cls.broadcast_div(&norm)?.to_vec2::<f32>()?)
    }

    /// A query's vector, with the bge query instruction in front.
    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbedError> {
        let q = format!("{BGE_QUERY_PREFIX}{query}");
        Ok(self.embed(&[&q])?.remove(0))
    }
}
