#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleDirection {
    Next,
    Prev,
}

pub fn normalize_workspace(id: u32, offset: u32) -> u32 {
    ((id - 1) % offset) + 1
}

pub fn cycle_target(base: u32, offset: u32, direction: CycleDirection, wrap: bool) -> u32 {
    match direction {
        CycleDirection::Next => {
            if wrap {
                (base % offset) + 1
            } else if base >= offset {
                offset
            } else {
                base + 1
            }
        }
        CycleDirection::Prev => {
            if wrap {
                ((base + offset - 2) % offset) + 1
            } else if base <= 1 {
                1
            } else {
                base - 1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CycleDirection, cycle_target, normalize_workspace};

    #[test]
    fn normalizes_workspace_ids_with_offset() {
        assert_eq!(normalize_workspace(1, 10), 1);
        assert_eq!(normalize_workspace(12, 10), 2);
    }

    #[test]
    fn cycles_next_with_wraparound() {
        assert_eq!(cycle_target(1, 10, CycleDirection::Next, true), 2);
        assert_eq!(cycle_target(10, 10, CycleDirection::Next, true), 1);
    }

    #[test]
    fn cycles_prev_with_wraparound() {
        assert_eq!(cycle_target(1, 10, CycleDirection::Prev, true), 10);
        assert_eq!(cycle_target(2, 10, CycleDirection::Prev, true), 1);
    }

    #[test]
    fn cycles_next_without_wraparound() {
        assert_eq!(cycle_target(9, 10, CycleDirection::Next, false), 10);
        assert_eq!(cycle_target(10, 10, CycleDirection::Next, false), 10);
    }

    #[test]
    fn cycles_prev_without_wraparound() {
        assert_eq!(cycle_target(2, 10, CycleDirection::Prev, false), 1);
        assert_eq!(cycle_target(1, 10, CycleDirection::Prev, false), 1);
    }
}
