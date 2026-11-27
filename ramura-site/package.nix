{
  writeShellApplication,
  gh,
  git,
  python3,
  playwright-driver,
}:
let
  python = python3.withPackages (ps: [ ps.playwright ]);
in
writeShellApplication {
  name = "ramura-site";
  runtimeInputs = [
    gh
    git
    python
  ];
  text = ''
    export PLAYWRIGHT_BROWSERS_PATH=${playwright-driver.browsers}
    export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
    url=https://github.com/ragassler/ragassler.github.io
    repo=''${XDG_CACHE_HOME:-$HOME/.cache}/ramura-site/repo
    out=''${RAMURA_FEEDBACK_DIR:-$HOME/ramura-feedback}

    if [[ ! -d $repo/.git ]]; then
      git clone --quiet "$url" "$repo"
    elif ! git -C "$repo" pull --ff-only --quiet; then
      echo "Could not update the site; showing the copy from $(git -C "$repo" log -1 --format=%cs)." >&2
    fi
    (cd "$repo" && python3 build.py >/dev/null)

    exec python3 -I ${./serve.py} \
      --site "$repo/_site" --out "$out" \
      --client ${../feedback/client} --shim ${./shim.js} \
      --rev "$(git -C "$repo" rev-parse --short HEAD)" --repo "$url" "$@"
  '';
}
