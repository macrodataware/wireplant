/// Returns the compile-time embedded default hydroelectric scenario.
pub fn get_default_scenario_toml() -> &'static str {
    include_str!("../../../scenarios/default_hydro.toml")
}
