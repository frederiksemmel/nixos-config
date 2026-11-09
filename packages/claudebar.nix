{
  lib,
  stdenvNoCC,
  fetchFromGitHub,
  makeWrapper,
  bashNonInteractive,
  coreutils,
  curl,
  gawk,
  gnugrep,
  gnused,
  jq,
  libnotify,
  util-linux,
}:

stdenvNoCC.mkDerivation (finalAttrs: {
  pname = "claudebar";
  version = "0.10.7";

  src = fetchFromGitHub {
    owner = "mryll";
    repo = "claudebar";
    tag = "v${finalAttrs.version}";
    hash = "sha256-FcT5FtaUGHId0AgQn529092xVr+vvvWTH+RM6fJjCBk=";
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
        gawk
        gnugrep
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
