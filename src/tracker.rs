use solana_sdk::signature::Signature;
use std::collections::HashMap;

pub struct Tracker {
    // Signature -> Sender Name
    pub pending_signatures: HashMap<Signature, String>,
}

impl Tracker {
    pub fn new() -> Self {
        Self {
            pending_signatures: HashMap::new(),
        }
    }

    pub fn record_signature(&mut self, signature: Signature, sender_name: String) {
        self.pending_signatures.insert(signature, sender_name);
    }
}
