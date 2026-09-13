use dioxus::prelude::*;
use omenic_web_components::statsview::StatsView;

#[component]
pub fn Stats() -> Element {
    rsx! {
        StatsView {}
    }
}
