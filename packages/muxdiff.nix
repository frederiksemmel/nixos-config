{
  lib,
  rustPlatform,
  fetchFromGitHub,
  makeWrapper,
  bat,
  gh,
  git,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "muxdiff";
  version = "0.2.0";

  src = fetchFromGitHub {
    owner = "Cammi-Technology";
    repo = "muxdiff";
    tag = "v${finalAttrs.version}";
    hash = "sha256-A+TFYRTvGQzPGSncwZy9UaYkc2OItIeTF9EK/buiQLA=";
  };

  cargoHash = "sha256-y1zEnxTErX5iP97Vt30ztFjRlxj+7EcBiJJ9cx0iWLo=";

  nativeBuildInputs = [ makeWrapper ];

  postFixup = ''
    wrapProgram $out/bin/muxdiff --suffix PATH : ${
      lib.makeBinPath [
        bat
        gh
        git
      ]
    }
  '';

  meta = {
    description = "Terminal multi-buffer diff with Helix keys and pull request review threads";
    homepage = "https://github.com/Cammi-Technology/muxdiff";
    license = lib.licenses.mit;
    mainProgram = "muxdiff";
  };
})
