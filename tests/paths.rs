use std::path::Path;
use warmup_rs::paths::{
    create_item_details_if_missing, ensure_dirs, is_appliance_os_release, Paths, Platform,
};

#[test]
fn detects_kodi_appliances_from_os_release() {
    assert!(is_appliance_os_release("NAME=\"CoreELEC\"\nID=coreelec\n"));
    assert!(is_appliance_os_release(
        "NAME=\"LibreELEC\"\nID=libreelec\n"
    ));
    assert!(is_appliance_os_release("ID=openelec\n"));
    assert!(is_appliance_os_release(
        "ID=custom\nID_LIKE=\"libreelec\"\n"
    ));
    assert!(!is_appliance_os_release(
        "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n"
    ));
    assert!(!is_appliance_os_release("VERSION_ID=coreelec\n"));
}

#[test]
fn desktop_defaults_live_under_output_dir() {
    let p = Paths::defaults(Platform::Desktop, Path::new("./output"));
    assert_eq!(p.item_details_db, Path::new("./output/ItemDetails.db"));
    assert_eq!(p.state_db, Path::new("./output/state.db"));
    assert_eq!(p.textures_db, Path::new("./output/Textures13.db"));
    assert_eq!(p.thumbnails_dir, Path::new("./output/images"));
}

#[test]
fn appliance_defaults_point_at_kodi() {
    let p = Paths::defaults(Platform::KodiAppliance, Path::new("./output"));
    assert!(p
        .item_details_db
        .starts_with("/storage/.kodi/userdata/addon_data"));
    assert_eq!(
        p.thumbnails_dir,
        Path::new("/var/media/CACHE_DRIVE/tmdb/Thumbnails")
    );
}

#[test]
fn creates_item_details_with_schema_and_seed_rows() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::defaults(Platform::Desktop, &dir.path().join("output"));
    ensure_dirs(&paths).unwrap();
    assert!(paths.thumbnails_dir.is_dir());
    assert!(create_item_details_if_missing(&paths.item_details_db).unwrap());
    assert!(!create_item_details_if_missing(&paths.item_details_db).unwrap());
    let conn = rusqlite::Connection::open(&paths.item_details_db).unwrap();
    let langs: i64 = conn
        .query_row("SELECT COUNT(*) FROM languages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(langs, 58);
    let art: i64 = conn
        .query_row("SELECT COUNT(*) FROM art", [], |r| r.get(0))
        .unwrap();
    assert_eq!(art, 0);
}
