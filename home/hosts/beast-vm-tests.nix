{
  lib,
  pkgs,
  ...
}:
let
  # blackjackeye's nixosTests (its `vmTests` output) are not built by NixCI. Each new main commit
  # gets one run here, reported as the GitHub status `beast/vmTests` on that commit.
  vmTestsMain = pkgs.writers.writeNuBin "blackjackeye-vm-tests-main" ''
    let dir = $"($env.HOME)/.local/state/blackjackeye-vm-tests"
    let url = "https://github.com/spadesvision/blackjackeye.git"
    let creds = ["-c" "credential.helper=" "-c" "credential.helper=!gh auth git-credential"]
    if not ($dir | path exists) {
      ^git ...$creds clone --quiet --filter=blob:none --no-checkout $url $dir
    }
    cd $dir
    ^git ...$creds fetch --quiet origin main
    let sha = (^git rev-parse origin/main | str trim)
    let last = $"($dir).last"
    if ($last | path exists) and ((open $last | str trim) == $sha) { return }
    # Recorded before the run, so a red main commit is tried once rather than every tick.
    $sha | save --force $last
    ^git checkout --quiet --force --detach $sha
    ^nu deployment/local-ci.nu vmTests --status
  '';
in
{
  systemd.user.services.blackjackeye-vm-tests = {
    Unit.Description = "blackjackeye vmTests on the newest main commit";
    Service = {
      Type = "oneshot";
      Slice = "batch.slice";
      ExecStart = lib.getExe vmTestsMain;
      Environment = [
        "PATH=/run/wrappers/bin:%h/.nix-profile/bin:/etc/profiles/per-user/%u/bin:/run/current-system/sw/bin:${
          lib.makeBinPath [
            pkgs.git
            pkgs.gh
            pkgs.nushell
          ]
        }"
      ];
      RuntimeMaxSec = "3h";
    };
  };
  systemd.user.timers.blackjackeye-vm-tests = {
    Unit.Description = "Checks for a new blackjackeye main commit to run the vmTests on";
    Timer.OnCalendar = "*:0/10";
    Install.WantedBy = [ "timers.target" ];
  };
}
