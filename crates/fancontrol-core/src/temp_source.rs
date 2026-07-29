//! Pick a sensible CPU temperature sensor id across Super I/O layouts.
//!
//! NCT668x EC uses names like `CPU` / `System`. Classic banked NCT (e.g. ROG
//! B550) uses `CPUTIN` / `PECI_0` / `SYSTIN`. Profiles often default to
//! `pawnio.0.temp.CPU`, which may be absent on banked chips — callers should
//! resolve via [`pick_cpu_temp_id`] or [`resolve_curve_temp_sensor`].

use std::collections::HashMap;

/// Priority for a short sensor name (after `pawnio.N.temp.` or bare id).
/// Lower is better. `None` = not a CPU-like source.
fn cpu_name_priority(name: &str) -> Option<u8> {
    let u = name.to_ascii_uppercase();
    match u.as_str() {
        "CPU" => Some(0),
        "PECI_0" | "PECI" => Some(1),
        "CPUTIN" => Some(2),
        "TCTL" | "TDIE" | "TCCD1" | "TCCD2" => Some(3),
        n if n.contains("CPU") => Some(4),
        n if n.contains("PECI") => Some(5),
        _ => None,
    }
}

/// Extract the short name from a sensor id (`pawnio.0.temp.CPUTIN` → `CPUTIN`).
pub fn temp_sensor_short_name(id: &str) -> &str {
    if let Some((_, name)) = id.rsplit_once("temp.") {
        if !name.is_empty() {
            return name;
        }
    }
    id.rsplit('.').next().unwrap_or(id)
}

/// Best CPU-like temperature id among `temps` keys (deterministic).
pub fn pick_cpu_temp_id(temps: &HashMap<String, f64>) -> Option<String> {
    let mut best: Option<(u8, String)> = None;
    for id in temps.keys() {
        let short = temp_sensor_short_name(id);
        let Some(prio) = cpu_name_priority(short) else {
            continue;
        };
        match &best {
            None => best = Some((prio, id.clone())),
            Some((bp, bid)) if prio < *bp || (prio == *bp && id < bid) => {
                best = Some((prio, id.clone()));
            }
            _ => {}
        }
    }
    best.map(|(_, id)| id).or_else(|| {
        if temps.contains_key("mock.cpu_temp") {
            Some("mock.cpu_temp".into())
        } else {
            None
        }
    })
}

/// Resolve which sensor to use for a control's curve:
/// 1. Explicit binding if present **and** available in `temps`
/// 2. Best CPU-like id among readings
/// 3. First sorted temp key (last resort)
pub fn resolve_curve_temp_sensor(
    bound: Option<&str>,
    temps: &HashMap<String, f64>,
) -> Option<String> {
    if let Some(b) = bound {
        if temps.contains_key(b) {
            return Some(b.to_string());
        }
    }
    if let Some(cpu) = pick_cpu_temp_id(temps) {
        return Some(cpu);
    }
    let mut keys: Vec<_> = temps.keys().cloned().collect();
    keys.sort();
    keys.into_iter().next()
}

/// Whether this short name / full id looks like a primary CPU package temp
/// (for UI `cpu_temp` / graph seed).
pub fn is_cpu_temp_candidate(id_or_name: &str) -> bool {
    let short = temp_sensor_short_name(id_or_name);
    cpu_name_priority(short).is_some()
}

/// Priority for UI seed (lower = better). Non-candidates get a high value.
pub fn cpu_temp_seed_priority(id_or_name: &str) -> u8 {
    let short = temp_sensor_short_name(id_or_name);
    cpu_name_priority(short).unwrap_or(100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_cputin_when_cpu_missing() {
        let mut t = HashMap::new();
        t.insert("pawnio.0.temp.CPUTIN".into(), 40.0);
        t.insert("pawnio.0.temp.SYSTIN".into(), 35.0);
        assert_eq!(
            pick_cpu_temp_id(&t).as_deref(),
            Some("pawnio.0.temp.CPUTIN")
        );
    }

    #[test]
    fn prefers_cpu_over_cputin() {
        let mut t = HashMap::new();
        t.insert("pawnio.0.temp.CPUTIN".into(), 40.0);
        t.insert("pawnio.0.temp.CPU".into(), 42.0);
        assert_eq!(pick_cpu_temp_id(&t).as_deref(), Some("pawnio.0.temp.CPU"));
    }

    #[test]
    fn prefers_peci_over_cputin() {
        let mut t = HashMap::new();
        t.insert("pawnio.0.temp.CPUTIN".into(), 40.0);
        t.insert("pawnio.0.temp.PECI_0".into(), 41.0);
        assert_eq!(
            pick_cpu_temp_id(&t).as_deref(),
            Some("pawnio.0.temp.PECI_0")
        );
    }

    #[test]
    fn stale_binding_falls_back_to_cputin() {
        let mut t = HashMap::new();
        t.insert("pawnio.0.temp.CPUTIN".into(), 40.0);
        let r = resolve_curve_temp_sensor(Some("pawnio.0.temp.CPU"), &t);
        assert_eq!(r.as_deref(), Some("pawnio.0.temp.CPUTIN"));
    }

    #[test]
    fn live_binding_kept() {
        let mut t = HashMap::new();
        t.insert("pawnio.0.temp.SYSTIN".into(), 30.0);
        t.insert("pawnio.0.temp.CPUTIN".into(), 40.0);
        let r = resolve_curve_temp_sensor(Some("pawnio.0.temp.SYSTIN"), &t);
        assert_eq!(r.as_deref(), Some("pawnio.0.temp.SYSTIN"));
    }
}
