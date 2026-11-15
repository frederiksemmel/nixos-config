CREATE TABLE "audio_prefs" (
    "profile_id" BIGINT NOT NULL,
    "headphones" TEXT NOT NULL CHECK ("headphones" IN ('unpaired', 'paired')),
    "headphones_address" TEXT,
    "headphones_listening" BOOLEAN,
    PRIMARY KEY ("profile_id"),
    CHECK (("headphones" = 'paired') = ("headphones_address" IS NOT NULL)),
    CHECK (("headphones" = 'paired') = ("headphones_listening" IS NOT NULL))
);
