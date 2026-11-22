{ lib, buildNpmPackage }:

buildNpmPackage {
  pname = "beachparty";
  version = "2026";

  src = lib.fileset.toSource {
    root = ./site;
    fileset = lib.fileset.unions [
      ./site/.npmrc
      ./site/package.json
      ./site/package-lock.json
      ./site/postcss.config.js
      ./site/tailwind.config.js
      ./site/tsconfig.json
      ./site/svelte.config.js
      ./site/vite.config.ts
      ./site/src
      ./site/static
    ];
  };

  npmDepsHash = "sha256-ugf8yW80Xieq/pbxqlNIbzbvwOWTV6JyTH2HfD6Usxk=";

  installPhase = ''
    runHook preInstall
    cp -r build $out
    runHook postInstall
  '';

  meta.description = "Silvester 2026 zu siebt in Sant Andreu de Llavaneres";
}
