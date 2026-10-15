//! Opt-in, per-.bus display adapter. The camera material stays the base variant.
use super::*;

pub const MODE: &str = "k_omsi_android_auto";
pub const TOGGLE: &str = "k_omsi_android_auto_toggle";
pub const PAGE: &str = "@k-omsi/android-auto";
pub const HTML: &str = include_str!("../../../../docs/examples/android-auto/index.html");

/// A sidecar beside a .bus: [android_auto], mesh path, camera texture, occurrence,
/// width, height. No sidecar means no change, including for AI vehicles.
pub(super) fn configure(bus: &Path, model: &mut Model) -> Result<bool> {
    let path = bus.with_extension("android-auto.cfg");
    if !path.is_file() { return Ok(false); }
    let cfg = omsi_cfg::CfgFile::read(&path)?;
    let mut r = cfg.reader();
    while let Some(k) = r.next_keyword() {
        if k != "android_auto" { continue; }
        let mesh = r.str().to_string();
        let texture = r.str().to_string();
        let occurrence = r.i32();
        let width = r.i32();
        let height = r.i32();
        anyhow::ensure!(occurrence >= 0 && (64..=2048).contains(&width) && (64..=2048).contains(&height), "invalid Android Auto dimensions or material occurrence");
        let vehicle = omsi_vehicle::Vehicle::load(bus)?;
        let model_path = omsi_cfg::resolve_path(vehicle.dir(), vehicle.model.as_deref().context("vehicle has no model")?);
        let mesh_path = omsi_cfg::resolve_path(model_path.parent().context("model has no directory")?, &mesh);
        let target = omsi_o3d::load_mesh(&mesh_path)?;
        anyhow::ensure!(target.materials.iter().filter(|m| m.texture.eq_ignore_ascii_case(&texture)).nth(occurrence as usize).is_some(), "Android Auto camera material not found in {}", mesh_path.display());

        // The camera may use only a cropped strip of its square reflection texture.
        // Carry that material's actual UV window to the HTML renderer; never change
        // the mesh UVs, since CCTV still uses the original camera material.
        let slot = target.materials.iter().enumerate().filter(|(_,m)| m.texture.eq_ignore_ascii_case(&texture)).nth(occurrence as usize).unwrap().0;
        let mut lo = glam::Vec2::splat(f32::INFINITY);
        let mut hi = glam::Vec2::splat(f32::NEG_INFINITY);
        for i in target.triangles.iter().filter(|t| t.material as usize == slot).flat_map(|t| t.indices) {
            let uv = target.vertices[i as usize].uv;
            lo = lo.min(uv); hi = hi.max(uv);
        }
        lo = lo.clamp(glam::Vec2::ZERO, glam::Vec2::ONE);
        hi = hi.clamp(glam::Vec2::ZERO, glam::Vec2::ONE);
        anyhow::ensure!(lo.is_finite() && hi.is_finite() && (hi-lo).min_element()>0.01,"invalid display UV window");
        apply(model, &mesh, &texture, occurrence, width, height)?;
        model.html_textures.last_mut().unwrap().path = format!("{PAGE}#uv={},{},{},{}",lo.x,lo.y,hi.x,hi.y);
        return Ok(true);
    }
    anyhow::bail!("{} has no [android_auto] block", path.display())
}

fn apply(model: &mut Model, mesh: &str, texture: &str, occurrence: i32, width: i32, height: i32) -> Result<()> {
    let norm = |s: &str| s.replace('\\', "/").to_ascii_lowercase();
    let matches: Vec<_> = model.meshes.iter().enumerate().filter(|(_, m)| norm(&m.file) == norm(mesh)).map(|(i, _)| i).collect();
    anyhow::ensure!(!matches.is_empty(), "Android Auto mesh not found: {mesh}");
    anyhow::ensure!(texture.to_ascii_lowercase().starts_with("reflexion") && texture.to_ascii_lowercase().ends_with(".bmp"), "Android Auto target must be a camera texture");
    for &i in &matches {
        let m = &model.meshes[i];
        anyhow::ensure!(m.mouse_event.is_none() && !m.materials.iter().any(|d| d.texture.eq_ignore_ascii_case(texture) && d.index == occurrence), "Android Auto target already has material overrides or a mouse event: {mesh}");
    }
    let script_index = model.script_textures.len();
    model.script_textures.push((width, height));
    model.html_textures.push(omsi_model::HtmlTextureDef { script_index, width, height, path: PAGE.into(), lit: false });
    for i in matches {
        let m = &mut model.meshes[i];
        m.mouse_event = Some(TOGGLE.into());
        m.materials.push(MaterialDef { texture: texture.into(), index: occurrence, change: Some((texture.into(), occurrence, MODE.into())), ..Default::default() });
        m.materials.push(MaterialDef { texture: texture.into(), index: occurrence, item: true, use_script_texture: Some(script_index as i32), ..Default::default() });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires installed Higer vehicle; set K_OMSI_HIGER_DIR"]
    fn installed_higer_camera_materials_and_model_adapter() {
        let dir = PathBuf::from(std::env::var("K_OMSI_HIGER_DIR").unwrap());
        for name in ["HigerHypers1611P.bus", "HigerHypers1611P_hiprest.bus"] {
            let vehicle = omsi_vehicle::Vehicle::load(&dir.join(name)).unwrap();
            assert_eq!(vehicle.cameras_reflexion.len(), 9);
            let path = omsi_cfg::resolve_path(&dir, vehicle.model.as_deref().unwrap());
            let mut model = Model::load(&path).unwrap();
            let before = model.clone();
            let camera = omsi_o3d::load_mesh(&omsi_cfg::resolve_path(path.parent().unwrap(), "Interior\\doorcamerascreen.o3d")).unwrap();
            assert_eq!(camera.materials.iter().filter(|m| m.texture == "reflexion8.bmp").count(), 1);
            let mut configured=before.clone();
            assert!(configure(&dir.join(name),&mut configured).unwrap());
            let uv=configured.html_textures.last().unwrap().path.split("#uv=").nth(1).unwrap().split(',').map(|s|s.parse::<f32>().unwrap()).collect::<Vec<_>>();
            assert!((uv[1]-0.20368588).abs()<0.00001);
            assert!((uv[3]-0.79631495).abs()<0.00001);
            assert_eq!(configured.script_textures.last(),Some(&(1600,1600)));
            apply(&mut model, "Interior/doorcamerascreen.o3d", "reflexion8.bmp", 0, 800, 480).unwrap();
            let changed: Vec<_> = model.meshes.iter().zip(&before.meshes).filter(|(a, b)| a != b).collect();
            assert_eq!(changed.len(), 2);
            for (a, b) in changed {
                assert_eq!(a.visible, b.visible);
                assert_eq!(a.materials[0].use_script_texture, None);
                assert_eq!(a.materials[1].use_script_texture, Some(before.script_textures.len() as i32));
            }
        }
    }
    #[test]
    fn preserves_camera_and_visibility_across_duplicate_screen_meshes() {
        let mut model = Model::default();
        for visible in ["elec_busbar_main", "antrieb_getr_gangwahl"] {
            model.meshes.push(omsi_model::MeshDef { file: "Interior\\doorcamerascreen.o3d".into(), visible: Some((visible.into(), 1.0)), ..Default::default() });
        }
        model.script_textures.push((32, 32));
        apply(&mut model, "interior/doorcamerascreen.o3d", "reflexion8.bmp", 0, 800, 480).unwrap();
        assert_eq!(model.html_textures[0].script_index, 1);
        for m in &model.meshes {
            assert!(m.visible.is_some());
            assert_eq!(m.materials[0].use_script_texture, None);
            assert_eq!(m.materials[0].texture, "reflexion8.bmp");
            assert_eq!(m.materials[1].use_script_texture, Some(1));
        }
        let original = model.clone();
        assert!(apply(&mut model, "Interior/doorcamerascreen.o3d", "reflexion8.bmp", 0, 800, 480).is_err());
        assert_eq!(model, original);
    }
}
