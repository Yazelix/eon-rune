use super::*;
use crate::PointerPress;
use crate::el::{col, el};

#[cfg(test)]
test_group! { input =>
mod app_ime;
}
#[cfg(test)]
test_group! { input =>
mod context_menu_key;
}
#[cfg(test)]
test_group! { paint =>
mod draw_on;
}
#[cfg(test)]
test_group! { app =>
mod edit;
}
#[cfg(test)]
test_group! { app =>
mod enter_in_instant;
}
#[cfg(test)]
test_group! { input =>
mod event_markers;
}
#[cfg(test)]
test_group! { layout =>
mod find_key;
}
#[cfg(test)]
test_group! { motion =>
mod fold_glide;
}
#[cfg(test)]
test_group! { paint =>
mod glyph_replay;
}
#[cfg(test)]
test_group! { paint =>
mod group_opacity;
}
#[cfg(test)]
test_group! { layout =>
mod hold_layout;
}
#[cfg(test)]
test_group! { layout =>
mod id_pruning;
}
#[cfg(test)]
test_group! { input =>
mod input_clipboard;
}
#[cfg(test)]
test_group! { layout =>
mod layout_hover;
}
#[cfg(test)]
test_group! { layout =>
mod layout_reads;
}
#[cfg(test)]
test_group! { layout =>
mod layout_rows;
}
#[cfg(test)]
test_group! { app =>
mod line_fills;
}
#[cfg(test)]
test_group! { native =>
mod loop_sleep;
}
#[cfg(test)]
test_group! { app =>
mod matched_in_place;
}
#[cfg(test)]
test_group! { motion =>
mod matched_scroll;
}
#[cfg(test)]
test_group! { app =>
mod matched_zoom;
}
#[cfg(test)]
test_group! { motion =>
mod no_motion;
}
#[cfg(test)]
test_group! { app =>
mod on_button;
}
#[cfg(test)]
test_group! { layout =>
mod one_snap;
}
#[cfg(test)]
test_group! { paint =>
mod picture_scale;
}
#[cfg(test)]
test_group! { selection =>
mod pointer_drag;
}
#[cfg(test)]
test_group! { selection =>
mod pointer_hit;
}
#[cfg(test)]
test_group! { input =>
mod press_moving;
}
#[cfg(test)]
test_group! { input =>
mod press_pointer;
}
#[cfg(test)]
test_group! { input =>
mod read_only_input;
}
#[cfg(test)]
test_group! { layout =>
mod relayout;
}
#[cfg(test)]
test_group! { layout =>
mod remount;
}
#[cfg(test)]
test_group! { motion =>
mod reveal_chain;
}
#[cfg(test)]
test_group! { motion =>
mod reveal_phase;
}
#[cfg(test)]
test_group! { motion =>
mod scroll_carried;
}
#[cfg(test)]
test_group! { motion =>
mod scroll_children;
}
#[cfg(test)]
test_group! { motion =>
mod scroll_growth;
}
#[cfg(test)]
test_group! { motion =>
mod scroll_hover;
}
#[cfg(test)]
test_group! { motion =>
mod scroll_retarget;
}
#[cfg(test)]
test_group! { motion =>
mod scroll_spring;
}
#[cfg(test)]
test_group! { app =>
mod secondary_drag;
}
#[cfg(test)]
test_group! { selection =>
mod secure_text;
}
#[cfg(test)]
test_group! { selection =>
mod selection_edge_walk;
}
#[cfg(test)]
test_group! { selection =>
mod selection_phase;
}
#[cfg(test)]
test_group! { selection =>
mod selection_phase_virtual;
}
#[cfg(test)]
test_group! { input =>
mod settle_input;
}
#[cfg(test)]
test_group! { motion =>
mod shimmer;
}
#[cfg(test)]
test_group! { app =>
mod span_box_overline;
}
#[cfg(test)]
test_group! { motion =>
mod springs;
}
#[cfg(test)]
test_group! { motion =>
mod stream_overflow;
}
#[cfg(test)]
test_group! { motion =>
mod stream_pace;
}
#[cfg(test)]
test_group! { app =>
mod text_align_y;
}
#[cfg(test)]
test_group! { motion =>
mod text_glide;
}
#[cfg(test)]
test_group! { app =>
mod text_indent;
}
#[cfg(test)]
test_group! { input =>
mod text_keys;
}
#[cfg(test)]
test_group! { paint =>
mod text_quads;
}
#[cfg(test)]
test_group! { layout =>
mod text_rebreak;
}
#[cfg(test)]
test_group! { input =>
mod titlebar_press;
}
#[cfg(test)]
test_group! { motion =>
mod unseen_entrances;
}
#[cfg(test)]
test_group! { native =>
mod window_wake;
}

thread_local! {
    /// Runs once inside the next frame, after layout and just before live values latch: a
    /// publication from another thread landing at the worst moment.
    pub(super) static MID_FRAME: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

fn duplicate_view(_: &(), _: &mut Cx<()>) -> El {
    col().children([el().key("card"), el().key("card")])
}
