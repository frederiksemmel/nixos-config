diesel::table! {
    profiles (id) {
        id -> BigInt,
        name -> Text,
        role -> Text,
        password_hash -> Nullable<Text>,
        color -> Nullable<Text>,
        created_at -> BigInt,
    }
}

diesel::table! {
    passkeys (id) {
        id -> BigInt,
        profile_id -> BigInt,
        credential_id -> Binary,
        rp_id -> Text,
        name -> Text,
        passkey -> Text,
        created_at -> BigInt,
        last_used_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    sessions (token_hash) {
        token_hash -> Binary,
        profile_id -> BigInt,
        created_at -> BigInt,
        last_seen_at -> BigInt,
        expires_at -> BigInt,
        label -> Nullable<Text>,
    }
}

diesel::table! {
    invites (token_hash) {
        token_hash -> Binary,
        profile_id -> BigInt,
        created_by -> Nullable<BigInt>,
        created_at -> BigInt,
        expires_at -> BigInt,
        used_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    files (path) {
        path -> Text,
        duration -> Nullable<Double>,
        credits -> Nullable<Double>,
        subtitle_delay -> Double,
        embedded_subtitles -> Nullable<BigInt>,
    }
}

diesel::table! {
    file_places (profile_id, path) {
        profile_id -> BigInt,
        path -> Text,
        stopped_at -> Nullable<Double>,
        last_played -> Nullable<BigInt>,
        finished -> Bool,
        audio_track -> Nullable<BigInt>,
        subtitle_track -> Nullable<BigInt>,
        subtitles_shown -> Nullable<Bool>,
    }
}

diesel::table! {
    show_places (profile_id, show_key) {
        profile_id -> BigInt,
        show_key -> Text,
        season -> BigInt,
        episode -> BigInt,
        finished -> Bool,
    }
}

diesel::table! {
    watching (profile_id, kind, key) {
        profile_id -> BigInt,
        kind -> Text,
        key -> Text,
        last_played -> BigInt,
    }
}

diesel::table! {
    dismissed (profile_id, path) {
        profile_id -> BigInt,
        path -> Text,
        at -> BigInt,
    }
}

diesel::table! {
    subtitle_settings (profile_id) {
        profile_id -> BigInt,
        settings -> Text,
    }
}

diesel::table! {
    audio_prefs (profile_id) {
        profile_id -> BigInt,
        output -> Text,
        headphones_address -> Nullable<Text>,
    }
}

diesel::table! {
    settings (key) {
        key -> Text,
        value -> Text,
    }
}

diesel::joinable!(passkeys -> profiles (profile_id));
diesel::joinable!(sessions -> profiles (profile_id));
diesel::joinable!(file_places -> profiles (profile_id));
diesel::joinable!(show_places -> profiles (profile_id));
diesel::joinable!(watching -> profiles (profile_id));
diesel::joinable!(dismissed -> profiles (profile_id));
diesel::joinable!(subtitle_settings -> profiles (profile_id));
diesel::joinable!(audio_prefs -> profiles (profile_id));

diesel::allow_tables_to_appear_in_same_query!(
    profiles,
    passkeys,
    sessions,
    invites,
    files,
    file_places,
    show_places,
    watching,
    dismissed,
    subtitle_settings,
    audio_prefs,
    settings,
);
