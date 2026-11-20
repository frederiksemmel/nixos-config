{
  lib,
  stdenvNoCC,
  fetchurl,
  makeWrapper,
  nodejs,
}:

stdenvNoCC.mkDerivation (finalAttrs: {
  pname = "machine0";
  version = "1.0.205";

  src = fetchurl {
    url = "https://registry.npmjs.org/@machine0/cli/-/cli-${finalAttrs.version}.tgz";
    hash = "sha512-9iKoRMeQAbkgdHQwbeWXZPtEyjp9KHJYn9t5u1frZ0VKp7eLWjwHUnAlZ6OXk9pY1M06sYFCIlVRJMvYW7eiGQ==";
  };

  nativeBuildInputs = [ makeWrapper ];

  installPhase = ''
    runHook preInstall
    mkdir -p $out/lib/machine0 $out/bin
    cp -r bin dist package.json $out/lib/machine0/
    makeWrapper ${lib.getExe nodejs} $out/bin/machine0 \
      --add-flags $out/lib/machine0/bin/entry.cjs
    runHook postInstall
  '';

  meta = {
    description = "Cloud VMs from the CLI";
    homepage = "https://machine0.io";
    license = lib.licenses.unfree;
    mainProgram = "machine0";
  };
})
