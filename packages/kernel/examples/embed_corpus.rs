//! E387: the in-kernel embedder (`embed` feature) on a corpus and on a
//! model of bge-small-en-v1.5's shape.
//!
//!   cargo run --release --features embed --example embed_corpus -- corpus MODEL DIR [PIN]
//!   cargo run --release --features embed --example embed_corpus -- bench WORK
//!
//! `corpus` reads `hybrid_kernel`'s files in DIR (`docs.tsv`, `queries.tsv`)
//! and writes `doc_whole.f32` and `queries.f32` from the model folder MODEL
//! (weights checked against PIN, a BLAKE3, when given), so `hybrid_kernel
//! DIR` scores the kernel's own vectors. If DIR already holds vectors from
//! another embedder (E306's Python run), they are kept as `*.f32.before`
//! and the cosine between the two is printed per file.
//!
//! `bench` times embedding with random weights in bge-small-en-v1.5's shape
//! (12 layers, 384 wide, 12 heads, 1,536 inner, 512 positions, a 30,522
//! word vocabulary): the cost of a forward pass does not depend on the
//! weights' values, so this is the model's speed on this machine, not its
//! quality. Texts of 64 to 512 tokens, in batches of 1 and 32.

use std::path::Path;
use std::time::Instant;

use candle_core::{DType, Device};
use candle_nn::{VarBuilder, VarMap};
use candle_transformers::models::bert::{BertModel, Config};
use ikam_kernel::embed::Embedder;
use ikam_kernel::vectors::{from_f32s, to_f32s};
use tokenizers::Tokenizer;
use tokenizers::models::wordpiece::WordPiece;
use tokenizers::normalizers::BertNormalizer;
use tokenizers::pre_tokenizers::bert::BertPreTokenizer;
use tokenizers::processors::bert::BertProcessing;

const BGE_SMALL_SHAPE: &str = r#"{"vocab_size": 30522, "hidden_size": 384, "num_hidden_layers": 12,
 "num_attention_heads": 12, "intermediate_size": 1536, "hidden_act": "gelu",
 "hidden_dropout_prob": 0.1, "max_position_embeddings": 512, "type_vocab_size": 2,
 "initializer_range": 0.02, "layer_norm_eps": 1e-12, "pad_token_id": 0,
 "classifier_dropout": null, "model_type": "bert"}"#;

fn write(path: &Path, vecs: &[Vec<f32>]) {
    let flat: Vec<f32> = vecs.iter().flatten().copied().collect();
    std::fs::write(path, from_f32s(&flat)).unwrap();
}

fn parity(before: &Path, after: &[Vec<f32>]) {
    let Ok(raw) = std::fs::read(before) else {
        return;
    };
    let old = to_f32s(&raw);
    let dim = after[0].len();
    if old.len() != after.len() * dim {
        println!("{}: other dimensions, not compared", before.display());
        return;
    }
    let mut cos: Vec<f32> = old
        .chunks_exact(dim)
        .zip(after)
        .map(|(a, b)| a.iter().zip(b).map(|(x, y)| x * y).sum())
        .collect();
    cos.sort_by(f32::total_cmp);
    println!(
        "{}: cosine to the earlier vectors min {:.5}, median {:.5}",
        before.display(),
        cos[0],
        cos[cos.len() / 2]
    );
}

fn corpus(model: &Path, dir: &Path, pin: Option<&str>) {
    let e = Embedder::open(model, pin, 512).unwrap();
    println!("weights {} ({} dims)", e.weights, e.dims);
    let docs: Vec<String> = std::fs::read_to_string(dir.join("docs.tsv"))
        .unwrap()
        .lines()
        .map(|l| l.split_once('\t').unwrap().1.replace('\u{1e}', "\n"))
        .collect();
    let queries: Vec<String> = std::fs::read_to_string(dir.join("queries.tsv"))
        .unwrap()
        .lines()
        .map(|l| l.splitn(5, '\t').nth(4).unwrap().to_string())
        .collect();
    let t = Instant::now();
    let mut dv = Vec::new();
    for chunk in docs.chunks(32) {
        let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
        dv.extend(e.embed(&refs).unwrap());
    }
    let dt = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let qv: Vec<Vec<f32>> = queries.iter().map(|q| e.embed_query(q).unwrap()).collect();
    let qt = t.elapsed().as_secs_f64();
    for (name, v) in [("doc_whole.f32", &dv), ("queries.f32", &qv)] {
        let p = dir.join(name);
        let before = dir.join(format!("{name}.before"));
        if p.exists() && !before.exists() {
            std::fs::rename(&p, &before).unwrap();
        }
        write(&p, v);
        parity(&before, v);
    }
    println!(
        "{} docs in {dt:.1} s ({:.1} a second), {} queries in {qt:.1} s ({:.1} ms each)",
        docs.len(),
        docs.len() as f64 / dt,
        queries.len(),
        qt * 1e3 / queries.len() as f64
    );
}

fn shaped_model(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("config.json"), BGE_SMALL_SHAPE).unwrap();
    let cfg: Config = serde_json::from_str(BGE_SMALL_SHAPE).unwrap();
    let vm = VarMap::new();
    BertModel::load(VarBuilder::from_varmap(&vm, DType::F32, &Device::Cpu), &cfg).unwrap();
    vm.save(dir.join("model.safetensors")).unwrap();
    // A word-level vocabulary of the model's size: one token a word.
    let mut words: Vec<String> = ["[PAD]", "[UNK]", "[CLS]", "[SEP]", "[MASK]"]
        .map(String::from)
        .to_vec();
    words.extend((words.len()..30522).map(|i| format!("w{i}")));
    let vocab = dir.join("vocab.txt");
    std::fs::write(&vocab, words.join("\n")).unwrap();
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

fn rss_mb() -> f64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines()
        .find(|l| l.starts_with("VmHWM:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<f64>().ok())
        .map_or(0.0, |kb| kb / 1024.0)
}

fn bench(work: &Path) {
    let model = work.join("bge-small-shape");
    if !model.join("model.safetensors").exists() {
        shaped_model(&model);
    }
    let t = Instant::now();
    let e = Embedder::open(&model, None, 512).unwrap();
    println!(
        "{{\"open_ms\":{:.0},\"dims\":{},\"peak_rss_mb\":{:.0}}}",
        t.elapsed().as_secs_f64() * 1e3,
        e.dims,
        rss_mb()
    );
    for tokens in [64usize, 128, 256, 510] {
        // tokens - 2 words plus [CLS] and [SEP].
        let text: String = (0..tokens - 2)
            .map(|i| format!("w{} ", 100 + i * 7 % 30000))
            .collect();
        for batch in [1usize, 32] {
            let texts: Vec<&str> = vec![text.as_str(); batch];
            e.embed(&texts).unwrap();
            let reps = if batch == 1 { 20 } else { 3 };
            let t = Instant::now();
            for _ in 0..reps {
                e.embed(&texts).unwrap();
            }
            let per_text = t.elapsed().as_secs_f64() / (reps * batch) as f64;
            println!(
                "{{\"tokens\":{tokens},\"batch\":{batch},\"ms_per_text\":{:.2},\"texts_per_s\":{:.1},\"peak_rss_mb\":{:.0}}}",
                per_text * 1e3,
                1.0 / per_text,
                rss_mb()
            );
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a[0].as_str() {
        "corpus" => corpus(
            Path::new(&a[1]),
            Path::new(&a[2]),
            a.get(3).map(String::as_str),
        ),
        "bench" => bench(Path::new(&a[1])),
        _ => panic!("corpus MODEL DIR [PIN] | bench WORK"),
    }
}
