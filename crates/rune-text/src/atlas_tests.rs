use super::*;

#[test]
fn explicit_dirty_rows_need_no_shelf_placement_and_survive_invalidation() {
    for rows in [(0, ATLAS_SIZE), (300, 512)] {
        let mut atlas = Atlas::new();
        atlas.data.resize((ATLAS_SIZE * ATLAS_SIZE) as usize, 0);
        atlas.dirty = Some(rows);
        assert_eq!(atlas.shelf.used_rows(), None);
        assert_eq!(atlas.page_rows(0), rows.1);
        atlas.resend_rows();
        assert_eq!(atlas.dirty, Some(rows));
        atlas.invalidate();
        assert_eq!(atlas.dirty, Some(rows));
        assert_eq!(atlas.page_rows(0), rows.1);
    }
}

#[test]
fn shelf_rows_and_pending_uploads_are_unioned_on_every_page() {
    let mut atlas = Atlas::new();
    let placed = atlas.put(2, 3, &[255; 6], 0, 0).unwrap();
    let used = placed.y + placed.h + 1;
    atlas.dirty = Some((used + 10, 700));
    let mut page = AtlasPage::new(1, 1);
    page.shelf.alloc(2, 3).unwrap();
    page.dirty = Some((used + 20, 900));
    atlas.pages.push(page);
    assert_eq!(atlas.page_rows(0), 700);
    assert_eq!(atlas.page_rows(1), 900);
    atlas.invalidate();
    assert_eq!(atlas.dirty, Some((0, 700)));
    assert_eq!(atlas.pages[0].dirty, Some((0, 900)));
    atlas.resend_rows();
    assert_eq!(atlas.dirty, Some((0, 700)));
    assert_eq!(atlas.pages[0].dirty, Some((0, 900)));
}

#[test]
fn empty_and_small_managed_atlases_keep_their_lazy_extent() {
    let mut atlas = Atlas::new();
    assert!(atlas.data.is_empty());
    assert_eq!(atlas.page_rows(0), 0);
    atlas.invalidate();
    assert_eq!(atlas.dirty, None);
    assert!(atlas.data.is_empty());
    let placed = atlas.put(2, 3, &[255; 6], 0, 0).unwrap();
    let used = placed.y + placed.h + 1;
    assert_eq!(atlas.page_rows(0), used);
    atlas.dirty = None;
    atlas.resend_rows();
    assert_eq!(atlas.dirty, Some((0, used)));
    atlas.invalidate();
    assert_eq!(atlas.page_rows(0), used);
    assert_eq!(atlas.dirty, Some((0, used)));
}
