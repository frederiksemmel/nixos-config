let
  frederik = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFYY+tQCwMzcg13wu+aJbcK0uA2EWS89akoAgxnW2yRL frederiksemmel@gmail.com";
  fstux = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIP3QuT7omO+HC1C5S24UjNC5WtnY9S343SELlJTRal9V root@fstux";
in
{
  "spadesvision_gdrive_token.age".publicKeys = [ frederik ];
  "spadesvision_gdrive_client_secret.age".publicKeys = [ frederik ];
  "pcloud_token.age".publicKeys = [ frederik ];
  "nixci_netrc.age".publicKeys = [
    frederik
    fstux
  ];
}
