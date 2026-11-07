let
  frederik = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFYY+tQCwMzcg13wu+aJbcK0uA2EWS89akoAgxnW2yRL frederiksemmel@gmail.com";
  fstux = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIP3QuT7omO+HC1C5S24UjNC5WtnY9S343SELlJTRal9V root@fstux";
  semmeltv5 = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINhTyGPbQD2wussVc/glbcGVkpOBSLy1Q6rhthB3jCvj";
in
{
  "spadesvision_gdrive_token.age".publicKeys = [ frederik ];
  "spadesvision_gdrive_client_secret.age".publicKeys = [ frederik ];
  "pcloud_token.age".publicKeys = [ frederik ];
  "spadesvision_ssh_config.age".publicKeys = [ frederik ];
  "spadesvision_ssh_config_agent.age".publicKeys = [ frederik ];
  "beast_linear_api_key.age".publicKeys = [ frederik ];
  "spadesvision_nixbuild_ssh_key.age".publicKeys = [
    frederik
    fstux
  ];
  "frederik_password_hash.age".publicKeys = [
    frederik
    fstux
  ];
  "root_password_hash.age".publicKeys = [
    frederik
    fstux
  ];
  "nixci_netrc.age".publicKeys = [
    frederik
    fstux
  ];
  "semmeltv_ssh_key.age".publicKeys = [ frederik ];
  "tmdb_token.age".publicKeys = [
    frederik
    semmeltv5
  ];
}
