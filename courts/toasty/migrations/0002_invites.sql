CREATE TABLE "invites" (
    "token_hash" TEXT NOT NULL,
    "created_at" BIGINT NOT NULL,
    "expires_at" BIGINT NOT NULL,
    "used_at" BIGINT,
    PRIMARY KEY ("token_hash")
);
