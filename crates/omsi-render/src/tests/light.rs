use super::*;

#[test]
fn the_wide_correction_is_for_the_day_and_a_night_keeps_the_old_metering() {
    let m = meter_tuning();
    // by day (the light model's pre-exposure under half a stop): a tunnel's darkness is made
    // up, most of it, over the whole reach
    assert_eq!(shortfall_adaptation(-1.0, m[3], m[0]), (m[3], METER_SHORTFALL_GAIN));
    // by night (past a pre-exposure of seven) the brightening is gone and the old share of
    // the shortfall is back: the light model's exposure is the eye's adaptation there
    assert_eq!(shortfall_adaptation(8.0, m[3], m[0]), (0.0, m[0]));
    // through the dusk it gives way smoothly, never above the day's reach, and the old
    // gentle reach (1.6 EV) is all there is by the time the sky is dark
    let mut last = (f32::MAX, f32::MAX);
    for k in 0..=40 {
        let (reach, gain) = shortfall_adaptation(k as f32 * 0.25, m[3], m[0]);
        assert!(reach <= last.0 + 1e-6 && gain <= last.1 + 1e-6 && reach <= m[3], "{k}: {reach} {gain}");
        last = (reach, gain);
    }
    assert!(shortfall_adaptation(3.0, m[3], m[0]).0 <= 1.6 + 1e-6);
}

/// A port of ssao.wgsl `bounce_pack` / `bounce_unpack` (which the scene shader repeats): the
/// colour of the bounced light survives the two channels to five bits, and its luminance
/// exactly.
#[test]
fn the_bounced_light_packs_and_unpacks() {
    const RANGE: f32 = 2.0;
    let pack = |e: [f32; 3]| {
        let y = 0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2];
        if y < 1e-6 {
            return (0.0, 0.0);
        }
        let u = (e[0] / y / RANGE).clamp(0.0, 1.0);
        let v = (e[2] / y / RANGE).clamp(0.0, 1.0);
        (y, (u * 31.0).round() * 32.0 + (v * 31.0).round())
    };
    let unpack = |(y, a): (f32, f32)| {
        let iu = (a / 32.0 + 0.001).floor();
        let iv = a - iu * 32.0;
        let r = iu / 31.0 * RANGE * y;
        let b = iv / 31.0 * RANGE * y;
        [r, ((y - 0.2126 * r - 0.0722 * b) / 0.7152).max(0.0), b]
    };
    for e in [[1.0, 1.0, 1.0], [0.9, 0.5, 0.2], [0.05, 0.4, 0.6], [0.5, 0.2, 0.1], [0.0, 0.0, 0.0]] {
        let d = unpack(pack(e));
        let (y, yd) = (0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2], 0.2126 * d[0] + 0.7152 * d[1] + 0.0722 * d[2]);
        assert!((y - yd).abs() <= 1e-4 + y * 0.02, "{e:?} -> {d:?}");
        for k in 0..3 {
            assert!((e[k] - d[k]).abs() <= 0.08 * RANGE * y.max(0.01) + 1e-4, "{e:?} -> {d:?}");
        }
    }
    // the same constant stands in both shaders
    assert!(include_str!("../ssao.wgsl").contains("const BOUNCE_RANGE: f32 = 2.0;"));
    assert!(include_str!("../shaders/scene/02_bindings.wgsl").contains("let r = iu / 31.0 * 2.0 * y;"));
}

#[test]
fn the_bounce_history_is_dropped_after_a_cut() {
    use glam::{DVec3, Mat4, Vec3};
    let vp = Mat4::perspective_rh(1.0, 1.7, 0.1, 1000.0);
    let view = Mat4::look_to_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
    let prev = Some((vp * view, DVec3::ZERO, (1280, 720), Vec3::ZERO, Vec3::NEG_Z));
    let valid = |prev, pos: Vec3, fwd: Vec3, size, dt| bounce_history(prev, view, DVec3::ZERO, size, pos, fwd, dt).1;
    assert_eq!(valid(prev, Vec3::new(0.1, 0.0, 0.0), Vec3::NEG_Z, (1280, 720), 1.0 / 60.0), 1.0);
    // another view (the cab to the outside), a jump, another size, a pause, a turn
    assert_eq!(valid(prev, Vec3::new(0.0, 0.0, -300.0), Vec3::NEG_Z, (1280, 720), 0.016), 0.0);
    assert_eq!(valid(prev, Vec3::ZERO, Vec3::Z, (1280, 720), 0.016), 0.0);
    assert_eq!(valid(prev, Vec3::ZERO, Vec3::NEG_Z, (1920, 1080), 0.016), 0.0);
    assert_eq!(valid(prev, Vec3::ZERO, Vec3::NEG_Z, (1280, 720), 2.0), 0.0);
    assert_eq!(valid(None, Vec3::ZERO, Vec3::NEG_Z, (1280, 720), 0.016), 0.0);
    // a standing camera maps every point to itself
    let (m, ok) = bounce_history(prev, view, DVec3::ZERO, (1280, 720), Vec3::ZERO, Vec3::NEG_Z, 0.016);
    assert_eq!(ok, 1.0);
    let c = Mat4::from_cols_array_2d(&m) * glam::Vec4::new(1.0, 2.0, -10.0, 1.0);
    let d = (vp * view) * glam::Vec4::new(1.0, 2.0, -10.0, 1.0);
    assert!((c - d).length() < 1e-3);
}

#[test]
fn the_auto_exposure_setting_scales_the_metering_and_off_holds_it() {
    let full = meter_tuning();
    // full: the metering as tuned
    let mut m = full;
    scale_metering(&mut m, auto_exposure_scale(1.0));
    assert_eq!(m, full);
    // off: no gain and no room to darken or brighten - the exposure is the light model's
    let mut off = full;
    scale_metering(&mut off, auto_exposure_scale(0.0));
    assert_eq!((off[0], off[2], off[3]), (0.0, 0.0, 0.0));
    // (the target, the bias and the night vision stay)
    assert_eq!((off[1], off[4], off[5]), (full[1], full[4], full[5]));
    // so a lamp-lit night picture far brighter than the target is not darkened at all
    let gap: f32 = -6.0;
    assert_eq!((gap * off[0]).clamp(-off[2], off[3]), 0.0);
    // half: half the gain and half the reach
    let mut half = full;
    scale_metering(&mut half, auto_exposure_scale(0.5));
    assert_eq!((half[0], half[2], half[3]), (full[0] * 0.5, full[2] * 0.5, full[3] * 0.5));
    // out of range or not a number: clamped, or the full metering
    assert_eq!((auto_exposure_scale(-1.0), auto_exposure_scale(2.0), auto_exposure_scale(f32::NAN)), (0.0, 1.0, 1.0));
}

#[test]
fn turned_off_the_night_exposure_keeps_to_the_district_not_the_lamps_in_view() {
    let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
    let (rest, artificial, district) = (0.5_f32, 0.3_f32, 0.2_f32);
    let base = rest - artificial;
    // full: the lamps and headlights in view decide (never below the district)
    assert!(near(night_adaptation_light(rest, artificial, 2.0, district, 1.0), base + 2.0));
    assert!(near(night_adaptation_light(rest, artificial, 0.05, district, 1.0), base + district));
    // off: the district's level alone, whatever is in view - no pumping
    for view in [0.05, 2.0, 50.0] {
        assert!(near(night_adaptation_light(rest, artificial, view, district, 0.0), base + district), "{view}");
    }
    // half: half of the way
    let half = night_adaptation_light(rest, artificial, 2.0, district, 0.5);
    assert!(near(half, base + district + (2.0 - district) * 0.5), "{half}");
}
