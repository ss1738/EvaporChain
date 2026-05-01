//! `compute_relation(a, b)` — exhaustive case analysis on the
//! `(start, end)` ordering between two intervals.

use std::cmp::Ordering;

use crate::interval::Interval;
use crate::relation::AllenRelation;

pub fn compute_relation(a: Interval, b: Interval) -> AllenRelation {
    use AllenRelation::*;
    use Ordering::*;
    let s = a.start.cmp(&b.start);
    let e = a.end.cmp(&b.end);
    let a_e_vs_b_s = a.end.cmp(&b.start);
    let a_s_vs_b_e = a.start.cmp(&b.end);

    // Strictly disjoint cases.
    if a_e_vs_b_s == Less {
        return Before;
    }
    if a_e_vs_b_s == Equal {
        return Meets;
    }
    if a_s_vs_b_e == Greater {
        return After;
    }
    if a_s_vs_b_e == Equal {
        return MetBy;
    }
    // Overlap cases — match on (s, e) ordering.
    match (s, e) {
        (Less, Less) => Overlaps, // a starts first, ends inside b
        (Equal, Less) => Starts,
        (Greater, Less) => During,
        (Greater, Equal) => Finishes,
        (Equal, Equal) => Equals,
        (Less, Equal) => FinishedBy,
        (Less, Greater) => Contains,
        (Equal, Greater) => StartedBy,
        (Greater, Greater) => OverlappedBy, // b starts first, ends inside a
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interval::Interval;

    fn iv(s: u64, e: u64) -> Interval {
        Interval::new(s, e).unwrap()
    }

    #[test]
    fn before() {
        assert_eq!(compute_relation(iv(1, 3), iv(5, 7)), AllenRelation::Before);
    }

    #[test]
    fn meets() {
        assert_eq!(compute_relation(iv(1, 5), iv(5, 7)), AllenRelation::Meets);
    }

    #[test]
    fn overlaps() {
        assert_eq!(
            compute_relation(iv(1, 5), iv(3, 7)),
            AllenRelation::Overlaps
        );
    }

    #[test]
    fn starts() {
        assert_eq!(compute_relation(iv(1, 3), iv(1, 7)), AllenRelation::Starts);
    }

    #[test]
    fn during() {
        assert_eq!(compute_relation(iv(2, 5), iv(1, 7)), AllenRelation::During);
    }

    #[test]
    fn finishes() {
        assert_eq!(
            compute_relation(iv(3, 7), iv(1, 7)),
            AllenRelation::Finishes
        );
    }

    #[test]
    fn equals() {
        assert_eq!(compute_relation(iv(1, 5), iv(1, 5)), AllenRelation::Equals);
    }

    #[test]
    fn finished_by() {
        assert_eq!(
            compute_relation(iv(1, 7), iv(3, 7)),
            AllenRelation::FinishedBy
        );
    }

    #[test]
    fn contains() {
        assert_eq!(
            compute_relation(iv(1, 7), iv(3, 5)),
            AllenRelation::Contains
        );
    }

    #[test]
    fn started_by() {
        assert_eq!(
            compute_relation(iv(1, 7), iv(1, 3)),
            AllenRelation::StartedBy
        );
    }

    #[test]
    fn overlapped_by() {
        assert_eq!(
            compute_relation(iv(3, 7), iv(1, 5)),
            AllenRelation::OverlappedBy
        );
    }

    #[test]
    fn met_by() {
        assert_eq!(compute_relation(iv(5, 7), iv(1, 5)), AllenRelation::MetBy);
    }

    #[test]
    fn after() {
        assert_eq!(compute_relation(iv(5, 7), iv(1, 3)), AllenRelation::After);
    }

    #[test]
    fn relation_inverse_consistency() {
        // For any (a, b), compute_relation(b, a) == compute_relation(a, b).inverse()
        let a = iv(2, 5);
        let b = iv(1, 7);
        assert_eq!(compute_relation(b, a), compute_relation(a, b).inverse());
    }
}
