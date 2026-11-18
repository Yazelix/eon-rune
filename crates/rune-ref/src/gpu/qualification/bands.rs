//! Beta exception: the frozen boundary defect's recurrence is intentionally accepted.
//! The ambiguity model depends only on Scene inputs; production ownership remains unfixed.
use super::{apply, guard, pixel};
use crate::{Frame, Primitive, Scene};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Decision {
    group: usize,
    include_below: bool,
    canonical_below: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Ambiguity {
    decisions: Vec<Vec<Option<Decision>>>,
    keys: Vec<[u32; 7]>,
    pixels: usize,
}

fn angled(p: &Primitive) -> bool {
    p.params[1] == 11.0 && p.uv[2] == 0.0 && p.uv[3] == 0.0
}

fn endpoint(raw: f32) -> f32 {
    if raw >= 1.5 { raw - 2.0 } else { raw }
}

pub(super) fn eligible(scene: &Scene) -> bool {
    super::target_eligible(scene, 2, true)
        && scene.primitives.len() <= 16
        && scene.primitives.iter().any(angled)
        && scene.primitives.iter().all(|p| {
            p.turn.is_none()
                && if angled(p) {
                    let lo = endpoint(p.params[0]);
                    let hi = endpoint(p.params[3]);
                    lo.is_finite() && hi.is_finite() && 0.0 <= lo && lo < hi && hi <= 1.0
                } else {
                    super::framebuffer_kind(p)
                }
        })
}

impl Ambiguity {
    fn new(scene: &Scene) -> Self {
        let pixels = (scene.width * scene.height) as usize;
        let mut result = Self {
            decisions: Vec::new(),
            keys: Vec::new(),
            pixels,
        };
        for p in &scene.primitives {
            let mut decisions = vec![None; pixels];
            if angled(p) {
                let center = [p.rect[0] + p.rect[2] * 0.5, p.rect[1] + p.rect[3] * 0.5];
                for (at, decision) in decisions.iter_mut().enumerate() {
                    let q =
                        [at as u32 % scene.width, at as u32 / scene.width].map(|v| v as f32 + 0.5);
                    let x = (q[0] - center[0]) * p.uv[0];
                    let y = (q[1] - center[1]) * p.uv[1];
                    let values = [0.5 + (x + y), (0.5 + x) + y, (0.5 + y) + x];
                    if values.iter().any(|v| !v.is_normal() || *v <= 0.0) {
                        continue;
                    }
                    let words = values.map(f32::to_bits);
                    let low = words[0].min(words[1]).min(words[2]);
                    let high = words[0].max(words[1]).max(words[2]);
                    if high - low > 1 {
                        continue;
                    }
                    let mut edges = [p.params[0], p.params[3]]
                        .into_iter()
                        .enumerate()
                        .filter_map(|(edge, raw)| {
                            let stop = endpoint(raw);
                            (raw >= 1.5
                                && stop > 0.0
                                && stop < 1.0
                                && f32::from_bits(low) < stop
                                && f32::from_bits(high) >= stop)
                                .then_some((edge, stop))
                        });
                    let Some((edge, stop)) = edges.next() else {
                        continue;
                    };
                    if edges.next().is_some() {
                        continue;
                    }
                    let key = [
                        p.rect[0].to_bits(),
                        p.rect[1].to_bits(),
                        p.rect[2].to_bits(),
                        p.rect[3].to_bits(),
                        p.uv[0].to_bits(),
                        p.uv[1].to_bits(),
                        stop.to_bits(),
                    ];
                    let group = group_index(&mut result.keys, key);
                    *decision = Some(Decision {
                        group,
                        include_below: edge == 1,
                        canonical_below: values[0] < stop,
                    });
                }
            }
            result.decisions.push(decisions);
        }
        result
    }
}

fn group_index(keys: &mut Vec<[u32; 7]>, key: [u32; 7]) -> usize {
    if let Some(at) = keys.iter().position(|value| *value == key) {
        return at;
    }
    keys.push(key);
    keys.len() - 1
}

pub(super) struct Choice {
    pub reference: [u8; 4],
    pub predicted: [u8; 4],
    pub active: bool,
}

pub(super) struct Inclusion {
    reference: Vec<u8>,
    predicted: Vec<u8>,
    owners: Vec<Option<usize>>,
}

pub(super) struct Qualification {
    ambiguity: Ambiguity,
    sides: Vec<Option<bool>>,
    included: Scene,
    pub canonical: Frame,
    pub before: Vec<u8>,
    pub operations: Vec<u32>,
    pub changed_decisions: u32,
}

impl Qualification {
    pub fn new(scene: &Scene, clear: &[u8]) -> Option<Self> {
        if !eligible(scene) {
            return None;
        }
        let ambiguity = Ambiguity::new(scene);
        let sides = vec![None; ambiguity.keys.len() * ambiguity.pixels];
        let mut included = scene.clone();
        for p in included.primitives.iter_mut().filter(|p| angled(p)) {
            p.params[0] = endpoint(p.params[0]);
            p.params[3] = endpoint(p.params[3]);
        }
        let mut canonical = Frame::new(scene.width, scene.height, true);
        canonical.rgba.copy_from_slice(clear);
        Some(Self {
            operations: vec![0; ambiguity.pixels],
            ambiguity,
            sides,
            included,
            canonical,
            before: clear.to_vec(),
            changed_decisions: 0,
        })
    }

    pub fn step(
        &mut self,
        scene: &Scene,
        index: usize,
        before: &[u8],
        gpu_before: &[u8],
    ) -> Option<Inclusion> {
        self.before.copy_from_slice(&self.canonical.rgba);
        if let Some(owners) = &mut self.canonical.owners {
            owners.fill(None);
        }
        apply(
            scene,
            index,
            &mut self.canonical.rgba,
            self.canonical.owners.as_deref_mut(),
        );
        if let Some(owners) = &self.canonical.owners {
            for (count, owner) in self.operations.iter_mut().zip(owners) {
                *count += u32::from(owner.is_some());
            }
        }
        if self.ambiguity.decisions[index].iter().all(Option::is_none) {
            return None;
        }
        let mut step = Inclusion {
            reference: before.to_vec(),
            predicted: gpu_before.to_vec(),
            owners: vec![None; self.ambiguity.pixels],
        };
        apply(
            &self.included,
            index,
            &mut step.reference,
            Some(&mut step.owners),
        );
        apply(&self.included, index, &mut step.predicted, None);
        Some(step)
    }

    pub fn choose(
        &mut self,
        index: usize,
        at: usize,
        before: [u8; 4],
        gpu_before: [u8; 4],
        actual: [u8; 4],
        included: &Inclusion,
    ) -> Result<Option<Choice>, usize> {
        let Some(decision) = self.ambiguity.decisions[index][at] else {
            return Ok(None);
        };
        let key = decision.group * self.ambiguity.pixels + at;
        let stored = self.sides[key];
        let first = stored.unwrap_or(decision.canonical_below);
        for below in [Some(first), stored.is_none().then_some(!first)]
            .into_iter()
            .flatten()
        {
            let choice = if below == decision.include_below {
                Choice {
                    reference: pixel(&included.reference, at),
                    predicted: pixel(&included.predicted, at),
                    active: included.owners[at].is_some(),
                }
            } else {
                Choice {
                    reference: before,
                    predicted: gpu_before,
                    active: false,
                }
            };
            if guard(
                before,
                gpu_before,
                choice.reference,
                choice.predicted,
                actual,
                choice.active,
            )
            .is_none()
            {
                self.sides[key] = Some(below);
                self.changed_decisions += u32::from(below != decision.canonical_below);
                return Ok(Some(choice));
            }
        }
        Err(decision.group)
    }

    pub fn ambiguous_pixels(&self) -> usize {
        (0..self.ambiguity.pixels)
            .filter(|at| self.ambiguity.decisions.iter().any(|p| p[*at].is_some()))
            .count()
    }

    pub fn side(&self, group: usize, at: usize) -> Option<bool> {
        self.sides[group * self.ambiguity.pixels + at]
    }

    pub fn selected_groups(&self) -> Vec<serde_json::Value> {
        self.ambiguity
            .keys
            .iter()
            .enumerate()
            .map(|(group, key)| {
                let sides =
                    &self.sides[group * self.ambiguity.pixels..(group + 1) * self.ambiguity.pixels];
                serde_json::json!({"group": group, "mapping_and_stop_bits": key,
                "below": sides.iter().filter(|v| **v == Some(true)).count(),
                "at_or_above": sides.iter().filter(|v| **v == Some(false)).count()})
            })
            .collect()
    }
}
