//! Explicit time stepping over an energy graph (E439): `M x'' = -dE/dx`
//! with a lumped (diagonal) mass, by the symplectic schemes that keep the
//! energy bounded over long runs: symplectic Euler (velocity first) and
//! velocity Verlet. Explicit Euler is here only as the non-symplectic
//! contrast. Opt-in; nothing else uses it.

use std::collections::BTreeMap;

use crate::eval::{Env, eval};
use crate::grad::grad;
use crate::ir::{Graph, V};
use crate::{Error, Tensor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    ExplicitEuler,
    SymplecticEuler,
    VelocityVerlet,
}

/// The potential's gradient graph, built once.
pub struct Dynamics {
    pub gradient: Graph,
    pub var: String,
    shape: Vec<usize>,
}

/// A state: positions, velocities, the potential at `x` and its gradient.
#[derive(Clone, Debug)]
pub struct State {
    pub x: Vec<f64>,
    pub v: Vec<f64>,
    pub potential: f64,
    force: Vec<f64>,
}

impl State {
    pub fn kinetic(&self, mass: &[f64]) -> f64 {
        0.5 * self.v.iter().zip(mass).map(|(v, m)| m * v * v).sum::<f64>()
    }
}

impl Dynamics {
    pub fn new(potential: &Graph, of: V, var: &str, shapes: &BTreeMap<String, Vec<usize>>) -> Result<Dynamics, Error> {
        let shape = shapes.get(var).ok_or_else(|| Error::MissingInput(var.into()))?.clone();
        Ok(Dynamics { gradient: grad(potential, shapes, of, &[var])?, var: var.into(), shape })
    }

    fn force(&self, env: &mut Env, x: &[f64]) -> Result<(f64, Vec<f64>), Error> {
        env.insert(self.var.clone(), Tensor { shape: self.shape.clone(), data: x.to_vec() });
        let out = eval(&self.gradient, env)?;
        Ok((out[0].item(), out[1].data.iter().map(|g| -g).collect()))
    }

    pub fn start(&self, env: &Env, x: Vec<f64>, v: Vec<f64>) -> Result<State, Error> {
        let mut env = env.clone();
        let (potential, force) = self.force(&mut env, &x)?;
        Ok(State { x, v, potential, force })
    }

    /// Advance `s` by one step `dt` (one gradient evaluation per step).
    pub fn step(&self, env: &mut Env, s: &mut State, mass: &[f64], dt: f64, scheme: Scheme) -> Result<(), Error> {
        if mass.len() != s.x.len() {
            return Err(Error::Shape(format!("{} masses for {} positions", mass.len(), s.x.len())));
        }
        // Velocity, then position, with the force at the old position
        // scaled by `a` (1 for symplectic Euler, 1/2 for Verlet's first half).
        let kick = |s: &mut State, a: f64| {
            for ((v, f), m) in s.v.iter_mut().zip(&s.force).zip(mass) {
                *v += a * dt * f / m;
            }
        };
        let drift = |s: &mut State| {
            for (x, v) in s.x.iter_mut().zip(&s.v) {
                *x += dt * v;
            }
        };
        match scheme {
            Scheme::ExplicitEuler => {
                drift(s);
                kick(s, 1.0);
            }
            Scheme::SymplecticEuler => {
                kick(s, 1.0);
                drift(s);
            }
            Scheme::VelocityVerlet => {
                kick(s, 0.5);
                drift(s);
            }
        }
        (s.potential, s.force) = self.force(env, &s.x)?;
        if scheme == Scheme::VelocityVerlet {
            kick(s, 0.5);
        }
        Ok(())
    }
}
