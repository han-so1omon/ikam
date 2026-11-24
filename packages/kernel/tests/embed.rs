//! E387: the in-kernel embedder on a tiny random BERT built here (no
//! weights are downloaded): vectors are unit length, deterministic, the
//! same alone or in a padded batch, truncated texts still embed, and a
//! wrong pin is refused.
#![cfg(feature = "embed")]

use std::path::Path;

use candle_core::{DType, Device};
use candle_nn::{VarBuilder, VarMap};
use candle_transformers::models::bert::{BertModel, Config};
use ikam_kernel::embed::{EmbedError, Embedder};
use tokenizers::Tokenizer;
use tokenizers::models::wordpiece::WordPiece;
use tokenizers::normalizers::BertNormalizer;
use tokenizers::pre_tokenizers::bert::BertPreTokenizer;
use tokenizers::processors::bert::BertProcessing;

const CONFIG: &str = r#"{"vocab_size": 48, "hidden_size": 32, "num_hidden_layers": 2,
 "num_attention_heads": 4, "intermediate_size": 64, "hidden_act": "gelu",
 "hidden_dropout_prob": 0.0, "max_position_embeddings": 64, "type_vocab_size": 2,
 "initializer_range": 0.02, "layer_norm_eps": 1e-12, "pad_token_id": 0,
 "classifier_dropout": null, "model_type": "bert"}"#;

fn tiny_model(dir: &Path) {
    std::fs::write(dir.join("config.json"), CONFIG).unwrap();
    let cfg: Config = serde_json::from_str(CONFIG).unwrap();
    let vm = VarMap::new();
    let vb = VarBuilder::from_varmap(&vm, DType::F32, &Device::Cpu);
    BertModel::load(vb, &cfg).unwrap();
    vm.save(dir.join("model.safetensors")).unwrap();
    let words = "[PAD] [UNK] [CLS] [SEP] [MASK] the a of to and in is it for on with as was at by an be this that from or are not file search graph store text query row data vector word fast slow kernel ikam".split(' ');
    let vocab = dir.join("vocab.txt");
    std::fs::write(&vocab, words.collect::<Vec<_>>().join("\n")).unwrap();
    let wp = WordPiece::from_file(vocab.to_str().unwrap())
        .unk_token("[UNK]".into())
        .build()
        .unwrap();
    let mut t = Tokenizer::new(wp);
    t.with_normalizer(Some(BertNormalizer::default()));
    t.with_pre_tokenizer(Some(BertPreTokenizer));
    t.with_post_processor(Some(BertProcessing::new(
        ("[SEP]".into(), 3),
        ("[CLS]".into(), 2),
    )));
    t.save(dir.join("tokenizer.json"), false).unwrap();
}

#[test]
fn vectors_are_unit_deterministic_and_batch_independent() {
    let dir = tempfile::tempdir().unwrap();
    tiny_model(dir.path());
    let e = Embedder::open(dir.path(), None, 512).unwrap();
    assert_eq!((e.dims, e.max_tokens), (32, 64));
    let texts = ["the kernel is fast", "a graph of rows and data", "search"];
    let batch = e.embed(&texts).unwrap();
    for (t, v) in texts.iter().zip(&batch) {
        assert_eq!(v.len(), 32);
        let n: f32 = v.iter().map(|x| x * x).sum();
        assert!((n - 1.0).abs() < 1e-5, "unit length");
        let alone = e.embed(&[t]).unwrap().remove(0);
        let dot: f32 = alone.iter().zip(v).map(|(a, b)| a * b).sum();
        assert!(dot > 0.99999, "padded batch changed {t:?}: {dot}");
    }
    assert_eq!(e.embed(&texts).unwrap(), batch, "deterministic");
    assert_ne!(batch[0], batch[1]);
    let long = "the file store ".repeat(200);
    assert_eq!(e.embed(&[&long]).unwrap()[0].len(), 32, "truncated");
    assert!(e.embed(&[]).unwrap().is_empty());
    assert_ne!(e.embed_query("search").unwrap(), batch[2], "query prefix");
}

#[test]
fn a_pinned_model_refuses_other_weights() {
    let dir = tempfile::tempdir().unwrap();
    tiny_model(dir.path());
    let w = Embedder::open(dir.path(), None, 512).unwrap().weights;
    assert!(Embedder::open(dir.path(), Some(&w.to_uppercase()), 512).is_ok());
    let other = "0".repeat(64);
    assert!(matches!(
        Embedder::open(dir.path(), Some(&other), 512),
        Err(EmbedError::Pin { .. })
    ));
}

#[test]
fn the_vector_id_changes_with_anything_that_changes_a_vector() {
    let dir = tempfile::tempdir().unwrap();
    tiny_model(dir.path());
    let a = Embedder::open(dir.path(), None, 512).unwrap();
    assert_eq!(a.id, Embedder::open(dir.path(), None, 512).unwrap().id);
    assert_ne!(a.id, a.weights);
    let short = Embedder::open(dir.path(), None, 16).unwrap();
    assert_eq!(short.weights, a.weights);
    assert_ne!(short.id, a.id, "max_tokens");
    let tok = dir.path().join("tokenizer.json");
    let mut t = std::fs::read_to_string(&tok).unwrap();
    t.push('\n');
    std::fs::write(&tok, t).unwrap();
    assert_ne!(
        Embedder::open(dir.path(), None, 512).unwrap().id,
        a.id,
        "tokenizer"
    );
}
