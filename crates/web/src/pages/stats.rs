use crate::components::statsview::StatsView;
use dioxus::prelude::*;

#[component]
pub fn Stats() -> Element {
    rsx! {
        StatsView {}
    }
}
