use std::collections::hash_set;

use std::collections::btree_set;

#[cfg(test)]
mod tests {
    use std::collections::BinaryHeap;
    use std::collections::HashSet;

    use std::cmp::Ordering;

    #[derive(Copy, Clone, Eq, PartialEq)]
    struct State {
        cost: usize,
        position: usize,
    }

    // The priority queue depends on `Ord`.
    // Explicitly implement the trait so the queue becomes a min-heap
    // instead of a max-heap.
    impl Ord for State {
        fn cmp(&self, other: &Self) -> Ordering {
            // Notice that the we flip the ordering on costs.
            // In case of a tie we compare positions - this step is necessary
            // to make implementations of `PartialEq` and `Ord` consistent.
            other
                .cost
                .cmp(&self.cost)
                .then_with(|| self.position.cmp(&other.position))
        }
    }

    // `PartialOrd` needs to be implemented as well.
    impl PartialOrd for State {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    #[test]
    fn test_hash_set() {
        let mut vikings = HashSet::new();
        vikings.insert(2);
        vikings.insert(6);
        vikings.insert(1);
        vikings.insert(0);
        vikings.insert(5);

        for x in &vikings {
            println!("{:?}", x);
        }
    }

    #[test]
    fn test_binary_heap() {
        let mut heap = BinaryHeap::new();
        heap.push(State{cost: 2, position: 3});
        heap.push(State{cost: 3, position: 3});
        heap.push(State{cost: 1, position: 3});
        heap.push(State{cost: 5, position: 3});
        heap.push(State{cost: 4, position: 3});

        loop{
            if heap.is_empty(){
                break;
            }

            let v = heap.pop().unwrap();
            println!("{:?}", v.cost);

        }

       
    }
}
