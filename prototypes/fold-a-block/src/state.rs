use ff::PrimeField;
use rand::Rng;

/// Configuration for the block circuit dimensions.
pub const NUM_ACCOUNTS: usize = 64;
pub const NUM_OBJECTS: usize = 64;
pub const NUM_TXS: usize = 50;
pub const DECAY_RATE: u64 = 5;
pub const INITIAL_ENERGY: u64 = 1000;

/// A single transfer instruction.
#[derive(Clone, Debug)]
pub struct Transfer {
    pub sender: usize,
    pub receiver: usize,
    pub amount: u64,
}

/// Per-block witness data representing one EvaporChain state transition.
#[derive(Clone, Debug)]
pub struct BlockWitness {
    pub epoch: u64,
    pub balances: Vec<u64>,
    pub energies: Vec<u64>,
    pub transfers: Vec<Transfer>,
    pub decay_rate: u64,
}

impl BlockWitness {
    /// Create the genesis state.
    pub fn genesis() -> Self {
        Self {
            epoch: 0,
            balances: vec![1_000_000; NUM_ACCOUNTS],
            energies: vec![INITIAL_ENERGY; NUM_OBJECTS],
            transfers: Vec::new(),
            decay_rate: DECAY_RATE,
        }
    }

    /// Generate a random block transition from the current state.
    pub fn random_next<R: Rng>(&self, rng: &mut R) -> Self {
        let mut new_balances = self.balances.clone();
        let mut transfers = Vec::with_capacity(NUM_TXS);

        for _ in 0..NUM_TXS {
            let sender = rng.gen_range(0..NUM_ACCOUNTS);
            let receiver = rng.gen_range(0..NUM_ACCOUNTS);
            if sender == receiver {
                continue;
            }
            let max_amount = new_balances[sender].min(1000);
            if max_amount == 0 {
                continue;
            }
            let amount = rng.gen_range(1..=max_amount);
            new_balances[sender] -= amount;
            new_balances[receiver] += amount;
            transfers.push(Transfer {
                sender,
                receiver,
                amount,
            });
        }

        let new_energies: Vec<u64> = self
            .energies
            .iter()
            .map(|e| e.saturating_sub(self.decay_rate))
            .collect();

        Self {
            epoch: self.epoch + 1,
            balances: new_balances,
            energies: new_energies,
            transfers,
            decay_rate: self.decay_rate,
        }
    }

    /// Compute a simple deterministic "hash" of the state.
    /// This is a placeholder for Poseidon — uses a field-friendly accumulation.
    pub fn state_commitment<F: PrimeField>(&self) -> F {
        let mut acc: u64 = self.epoch.wrapping_mul(31);
        for (i, b) in self.balances.iter().enumerate() {
            acc = acc.wrapping_add(b.wrapping_mul((i as u64).wrapping_add(7)));
        }
        for (i, e) in self.energies.iter().enumerate() {
            acc = acc.wrapping_add(e.wrapping_mul((i as u64).wrapping_add(13)));
        }
        F::from(acc)
    }

    /// Count how many objects have evaporated (energy == 0).
    pub fn evaporated_count(&self) -> usize {
        self.energies.iter().filter(|&&e| e == 0).count()
    }
}
