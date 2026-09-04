use crate::components::statsview::StatsView;
use crate::mock;
use dioxus::prelude::*;

#[component]
pub fn Stats() -> Element {
    let data = mock::mock_stats();
    rsx! {
        StatsView { data: data }
    }
}
