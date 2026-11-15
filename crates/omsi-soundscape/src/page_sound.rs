//! The sound of the videos on the player's bus's pages (`[htmltexture]` with a `<video>` that
//! is not `muted`): each comes from the screen that shows its page, heard in the bus like its
//! own sounds and through the bodywork from outside, like the bus radio.
//!
//! The page decides what is heard (`HtmlRenderer::sounds`); this follows its list each frame:
//! a stream with a new id gets a voice, a voice whose id is gone is stopped.

use glam::Vec3;
use omsi_audio::stream::StreamBuf;
use omsi_audio::{AudioEngine, VoiceId, VoiceParams};
use std::collections::HashMap;
use std::sync::Arc;

/// Distance (m) of full volume from the screen inside the bus.
const RANGE_INSIDE: f32 = 2.5;
/// Gain and low-pass of a page's sound heard from outside the bus.
const OUTSIDE_GAIN: f32 = 0.35;
const OUTSIDE_LOWPASS: f32 = 700.0;

#[derive(Default)]
pub struct PageSounds {
    /// Playing voices by the page sound's id.
    voices: HashMap<u64, VoiceId>,
    /// Where each page's screen is, in its mesh's frame: (vehicle type, script texture index)
    /// -> (mesh, centre), None when no mesh shows the page.
    screens: HashMap<(usize, usize), Option<(usize, Vec3)>>,
}

impl PageSounds {
    /// Follow the pages of the player's bus: `inside` when the camera is in it; `paused`
    /// keeps the voices silent.
    pub fn update(&mut self, audio: &AudioEngine, v: &omsi_sim::VehicleInstance, inside: bool, paused: bool) {
        let mut seen: Vec<u64> = Vec::new();
        for page in &v.html_textures {
            let sounds = page.sounds();
            if sounds.is_empty() {
                continue;
            }
            let at = self.screen_point(v, page.script_index);
            for s in sounds {
                seen.push(s.id);
                let gain = if paused { 0.0 } else { s.gain.clamp(0.0, 1.0) };
                let params = if inside {
                    VoiceParams { gain, position: Some(at), range: RANGE_INSIDE, doppler: false, ..Default::default() }
                } else {
                    VoiceParams { gain: gain * OUTSIDE_GAIN, position: Some(at), range: 4.0, doppler: false, lowpass_hz: OUTSIDE_LOWPASS, ..Default::default() }
                };
                match self.voices.get(&s.id) {
                    Some(&voice) => audio.set_params(voice, params),
                    None => {
                        let Ok(stream) = s.stream.clone().downcast::<StreamBuf>() else { continue };
                        log::debug!("htmltexture #{}: the page's video sound plays", page.script_index);
                        self.voices.insert(s.id, audio.play_stream(stream, params));
                    }
                }
            }
        }
        self.voices.retain(|id, voice| {
            let keep = seen.contains(id);
            if !keep {
                audio.stop(*voice);
            }
            keep
        });
    }

    /// No bus of one's own: every page's sound stops.
    pub fn stop(&mut self, audio: &AudioEngine) {
        for (_, voice) in self.voices.drain() {
            audio.stop(voice);
        }
    }

    /// The world point of the screen that shows page `index`: the middle of the triangles
    /// of the mesh's material that uses that script texture; the driver's place when no
    /// mesh shows it.
    fn screen_point(&mut self, v: &omsi_sim::VehicleInstance, index: usize) -> Vec3 {
        let key = (Arc::as_ptr(&v.ty) as usize, index);
        let screen = *self.screens.entry(key).or_insert_with(|| find_screen(v, index));
        match screen {
            Some((mesh, centre)) => v.position.as_vec3() + v.mesh_local_transform(mesh).transform_point3(centre),
            None => {
                let eye = v.ty.def.cameras_driver.first().map(|c| Vec3::from(c.pos)).unwrap_or(Vec3::new(0.0, 0.0, 1.8));
                v.world_transform().transform_point3(eye)
            }
        }
    }
}

fn find_screen(v: &omsi_sim::VehicleInstance, index: usize) -> Option<(usize, Vec3)> {
    for (i, vm) in v.ty.meshes.iter().enumerate() {
        let def = &v.ty.model.meshes[vm.def_index];
        let slots: Vec<usize> = def
            .materials
            .iter()
            .filter(|m| m.use_script_texture.is_some_and(|n| n.max(0) as usize == index))
            .filter_map(|m| omsi_sim::vehicle::override_slot(&vm.materials, m))
            .collect();
        if slots.is_empty() {
            continue;
        }
        let (mut sum, mut n) = (Vec3::ZERO, 0u32);
        for &(first, count, slot) in &vm.data.ranges {
            if !slots.contains(&(slot as usize)) {
                continue;
            }
            for &ix in vm.data.indices.iter().skip(first as usize).take(count as usize) {
                if let Some(p) = vm.data.positions.get(ix as usize) {
                    sum += *p;
                    n += 1;
                }
            }
        }
        if n > 0 {
            return Some((i, sum / n as f32));
        }
    }
    None
}
