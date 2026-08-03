//! Shared Training workspace controls.

use dioxus::prelude::*;

pub static TRAINING_CLUSTER_SCOPE: GlobalSignal<bool> = Signal::global(|| false);
pub static TRAINING_REFRESH: GlobalSignal<u32> = Signal::global(|| 0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlacementAvailability {
    #[default]
    Loading,
    Available,
    Missing,
    RegistryUnavailable,
}

pub static TRAINING_PLACEMENT_AVAILABILITY: GlobalSignal<PlacementAvailability> =
    Signal::global(PlacementAvailability::default);

pub fn placement_availability<T, E>(
    node_state: Option<&Result<Vec<T>, E>>,
) -> PlacementAvailability {
    match node_state {
        None => PlacementAvailability::Loading,
        Some(Err(_)) => PlacementAvailability::RegistryUnavailable,
        Some(Ok(nodes)) if nodes.is_empty() => PlacementAvailability::Missing,
        Some(Ok(_)) => PlacementAvailability::Available,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_availability_preserves_missing_and_failed_states() {
        let loading: Option<&Result<Vec<i32>, &str>> = None;
        let missing = Ok::<Vec<i32>, &str>(Vec::new());
        let available = Ok::<Vec<i32>, &str>(vec![0]);
        let failed = Err::<Vec<i32>, &str>("registry failed");

        assert_eq!(
            placement_availability(loading),
            PlacementAvailability::Loading
        );
        assert_eq!(
            placement_availability(Some(&missing)),
            PlacementAvailability::Missing
        );
        assert_eq!(
            placement_availability(Some(&available)),
            PlacementAvailability::Available
        );
        assert_eq!(
            placement_availability(Some(&failed)),
            PlacementAvailability::RegistryUnavailable
        );
    }
}
