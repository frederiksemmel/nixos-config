{
  lib,
  stdenvNoCC,
  fetchFromGitHub,
  makeWrapper,
  bashNonInteractive,
  coreutils,
  curl,
  gnused,
  jq,
  libnotify,
  util-linux,
}:

stdenvNoCC.mkDerivation (finalAttrs: {
  pname = "claudebar";
  version = "0.8.1";

  src = fetchFromGitHub {
    owner = "mryll";
    repo = "claudebar";
    tag = "v${finalAttrs.version}";
    hash = "sha256-te/ktnawRkHbUlENTmy/FS6eLCWXVzzr3od/2v9S7ow=";
  };

  nativeBuildInputs = [ makeWrapper ];
  buildInputs = [ bashNonInteractive ];

  postPatch = ''
    substituteInPlace claudebar \
      --replace-fail 'notify-send -u normal' 'notify-send -a Claudio -u normal' \
      --replace-fail '"Claude Usage"' '"Claudio usage"' \
      --replace-fail '_tip_title="Claude ''${plan}"' '_tip_title="Claudio ''${plan}"'
  '';

  dontBuild = true;

  installFlags = [ "PREFIX=${placeholder "out"}" ];

  # After patchShebangs, so the wrapper never points at an unpatched script.
  postFixup = ''
    wrapProgram $out/bin/claudebar --prefix PATH : ${
      lib.makeBinPath [
        coreutils # GNU date, cut, mktemp, timeout
        curl
        gnused
        jq
        libnotify # notify-send, for rate-limit warnings
        util-linux # flock, column
      ]
    }
  '';

  meta = {
    description = "Claude AI plan usage widget for Waybar";
    homepage = "https://github.com/mryll/claudebar";
    license = lib.licenses.mit;
    mainProgram = "claudebar";
    platforms = lib.platforms.unix;
  };
})
