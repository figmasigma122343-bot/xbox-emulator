use std::path::PathBuf;

const REQUIRED: &[&str] = &[
    "fortnite/catalog.json",
    "fortnite/keychain.json",
    "fortnite/contentpages.json",
    "fortnite/epic-settings.json",
    "fortnite/sdkv1.json",
    "fortnite/timeline.json",
    "fortnite/tournament.json",
    "fortnite/tournamentandhistory.json",
    "fortnite/history.json",
    "fortnite/privacy.json",
    "fortnite/quests_athena_s14.json",
    "fortnite/profile_athena.json",
    "fortnite/profile_common_core.json",
    "fortnite/profile_common_public.json",
    "fortnite/profile_creative.json",
    "fortnite/profile_collections.json",
    "fortnite/profile_metadata.json",
    "fortnite/profile_campaign.json",
    "fortnite/DefaultEngine.ini",
    "fortnite/DefaultGame.ini",
    "fortnite/DefaultInput.ini",
    "fortnite/DefaultRuntimeOptions.ini",
    "fortnite/video/init.mp4",
    "fortnite/video/seg0.m4s",
    "fortnite/video/audio_init.mp4",
    "fortnite/video/audio_seg0.m4s",
    "xdefiant/ubiservices_parameters.json",
    "trackmania/core_routes.json",
];

#[test]
fn build_leaves_every_required_backend_file_in_place() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("backend_data");
    for relative in REQUIRED {
        let path = root.join(relative);
        assert!(path.is_file(), "missing {}; compat-common/build.rs should create a placeholder", path.display());
        if relative.ends_with(".json") {
            let text = std::fs::read_to_string(&path).unwrap();
            serde_json::from_str::<serde_json::Value>(&text).unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));
        }
    }
}
