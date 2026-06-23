//! Baseline agents + an episode runner (spec 08 §9). An agent picks which keys to
//! hold next step from the typed observation; baselines turn "every authored game
//! is a benchmark" into concrete numbers (a no-op/random floor, a scripted ceiling).

use crate::{FromState, SpectrumEnv, StateView};

/// An agent: choose which keys to hold for the next step, given the observed state.
pub trait Agent {
    /// Keys to hold (by character, e.g. `'o'`/`'p'`/`'q'`/`'a'`).
    fn act(&mut self, view: &StateView) -> Vec<char>;
    /// Reset any per-episode internal state (e.g. the agent's own RNG).
    fn reset(&mut self) {}
}

/// Presses nothing — the trivial floor baseline.
#[derive(Default)]
pub struct NoOpAgent;

impl Agent for NoOpAgent {
    fn act(&mut self, _view: &StateView) -> Vec<char> {
        Vec::new()
    }
}

/// Presses a random direction each step. Deterministic given its seed (its own
/// xorshift), so episodes are reproducible.
pub struct RandomAgent {
    seed: u32,
    state: u32,
}

impl RandomAgent {
    pub fn new(seed: u32) -> Self {
        let s = seed | 1;
        RandomAgent { seed: s, state: s }
    }
    fn next(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }
}

impl Agent for RandomAgent {
    fn act(&mut self, _view: &StateView) -> Vec<char> {
        let dirs = ['o', 'p', 'q', 'a']; // left, right, up, down
        vec![dirs[(self.next() % 4) as usize]]
    }
    fn reset(&mut self) {
        self.state = self.seed;
    }
}

/// Greedily steps toward `(tx, ty)` by comparing the player's position to the
/// target's — a strong scripted ceiling for the "reach" task. Reads only typed
/// fields off the view (no pixels), exactly the kind of probe the symbol map
/// makes free.
#[derive(Default)]
pub struct ScriptedAgent;

impl Agent for ScriptedAgent {
    fn act(&mut self, view: &StateView) -> Vec<char> {
        let (px, py, tx, ty) = (
            view.u16("px"),
            view.u16("py"),
            view.u16("tx"),
            view.u16("ty"),
        );
        let mut keys = Vec::new();
        if tx > px {
            keys.push('p'); // right
        } else if tx < px {
            keys.push('o'); // left
        }
        if ty > py {
            keys.push('a'); // down
        } else if ty < py {
            keys.push('q'); // up
        }
        keys
    }
}

/// Run one episode: `reset` the env (bit-exact) and the agent, then for up to
/// `steps` steps let the agent act and accumulate the host game's reward. Returns
/// total reward. Reproducible — same env + same agent ⇒ same number.
pub fn run_episode<G, A>(
    env: &mut SpectrumEnv,
    agent: &mut A,
    steps: usize,
    frames_per_step: usize,
) -> i64
where
    G: speccy_sdk::Game + FromState,
    A: Agent,
{
    env.reset();
    agent.reset();
    let mut total = 0i64;
    for _ in 0..steps {
        let keys = agent.act(&env.view());
        let t = env.step_game::<G>(&keys, frames_per_step);
        total += t.reward as i64;
        if t.done {
            break;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_steers_toward_target() {
        // Target down-right of the player → press right + down.
        let v = StateView::from_pairs(&[("px", 5), ("py", 5), ("tx", 10), ("ty", 9)]);
        let keys = ScriptedAgent.act(&v);
        assert!(keys.contains(&'p') && keys.contains(&'a'));
        // Target up-left → left + up.
        let v = StateView::from_pairs(&[("px", 20), ("py", 20), ("tx", 3), ("ty", 2)]);
        let keys = ScriptedAgent.act(&v);
        assert!(keys.contains(&'o') && keys.contains(&'q'));
        // On the target → no keys.
        let v = StateView::from_pairs(&[("px", 7), ("py", 7), ("tx", 7), ("ty", 7)]);
        assert!(ScriptedAgent.act(&v).is_empty());
    }

    #[test]
    fn random_agent_is_seed_reproducible() {
        let mut a = RandomAgent::new(99);
        let v = StateView::from_pairs(&[]);
        let first: Vec<char> = (0..10).flat_map(|_| a.act(&v)).collect();
        a.reset();
        let again: Vec<char> = (0..10).flat_map(|_| a.act(&v)).collect();
        assert_eq!(first, again, "reset replays the same choices");
    }
}
