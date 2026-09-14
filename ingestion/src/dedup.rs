use std::collections::{HashSet, VecDeque};
pub fn remember_signature(
    signature: &str,
    seen: &mut HashSet<String>,
    queue: &mut VecDeque<String>,
) -> bool {
    if seen.contains(signature) {
        return false;
    }

    let signature = signature.to_string();

    seen.insert(signature.clone());

    queue.push_back(signature);

    if queue.len() > 10_000 {
        if let Some(old) = queue.pop_front() {
            seen.remove(&old);
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_fifo_deduplication() {
        let (mut set, mut queue) = (HashSet::new(), VecDeque::new());
        assert!(remember_signature("first", &mut set, &mut queue));
        assert!(!remember_signature("first", &mut set, &mut queue));
        for i in 0..10_000 {
            remember_signature(&i.to_string(), &mut set, &mut queue);
        }
        assert_eq!(set.len(), 10_000);
        assert_eq!(queue.len(), 10_000);
        assert!(!set.contains("first"));
    }
}

/// Shared by protocol listeners so routed signatures enter the queue only once.
#[derive(Default)]
pub struct EarlyDedup {
    seen: HashSet<String>,
    order: VecDeque<String>,
}
impl EarlyDedup {
    pub fn admit(&mut self, signature: &str) -> bool {
        remember_signature(signature, &mut self.seen, &mut self.order)
    }
}
