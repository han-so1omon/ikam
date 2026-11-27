//! Tensors as content. Values are f64 in little-endian IEEE 754 bits with
//! one canonical NaN, so a tensor's content id is the same on every
//! platform: storing float bits is portable; only computing them varies
//! (E409).
//!
//! Encoding: `ikT1`, dtype (1 = f64), rank, each dimension as u64 LE, then
//! the values.

use crate::Error;
use crate::pinned::canon;

#[derive(Clone, Debug)]
pub struct Tensor {
    pub shape: Vec<usize>,
    pub data: Vec<f64>,
}

const MAGIC: &[u8; 4] = b"ikT1";
const F64: u8 = 1;

impl Tensor {
    pub fn new(shape: Vec<usize>, data: Vec<f64>) -> Result<Tensor, Error> {
        if shape.iter().product::<usize>() != data.len() {
            return Err(Error::Shape(format!("shape {shape:?} holds {} values", data.len())));
        }
        Ok(Tensor { shape, data })
    }

    pub fn scalar(v: f64) -> Tensor {
        Tensor { shape: vec![], data: vec![v] }
    }

    pub fn vector(data: Vec<f64>) -> Tensor {
        Tensor { shape: vec![data.len()], data }
    }

    pub fn zeros(shape: &[usize]) -> Tensor {
        Tensor { shape: shape.to_vec(), data: vec![0.0; shape.iter().product()] }
    }

    pub fn is_scalar(&self) -> bool {
        self.shape.is_empty()
    }

    /// The single value of a scalar (or of any one-element tensor).
    pub fn item(&self) -> f64 {
        self.data[0]
    }

    /// Same shape and the same bits in every value.
    pub fn bits_eq(&self, other: &Tensor) -> bool {
        self.shape == other.shape
            && self.data.len() == other.data.len()
            && self
                .data
                .iter()
                .zip(&other.data)
                .all(|(a, b)| canon(*a).to_bits() == canon(*b).to_bits())
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(6 + 8 * (self.shape.len() + self.data.len()));
        out.extend_from_slice(MAGIC);
        out.push(F64);
        out.push(self.shape.len() as u8);
        for d in &self.shape {
            out.extend_from_slice(&(*d as u64).to_le_bytes());
        }
        for v in &self.data {
            out.extend_from_slice(&canon(*v).to_bits().to_le_bytes());
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Tensor, Error> {
        let bad = || Error::Decode("tensor");
        if bytes.len() < 6 || &bytes[..4] != MAGIC || bytes[4] != F64 {
            return Err(bad());
        }
        let rank = bytes[5] as usize;
        let mut at = 6;
        let mut shape = Vec::with_capacity(rank);
        for _ in 0..rank {
            let b = bytes.get(at..at + 8).ok_or_else(bad)?;
            shape.push(u64::from_le_bytes(b.try_into().unwrap()) as usize);
            at += 8;
        }
        let n: usize = shape.iter().product();
        if bytes.len() != at + 8 * n {
            return Err(bad());
        }
        let data = bytes[at..]
            .chunks_exact(8)
            .map(|c| f64::from_bits(u64::from_le_bytes(c.try_into().unwrap())))
            .collect();
        Ok(Tensor { shape, data })
    }
}
