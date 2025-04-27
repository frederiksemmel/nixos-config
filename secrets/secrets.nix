let
  frederik = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFYY+tQCwMzcg13wu+aJbcK0uA2EWS89akoAgxnW2yRL frederiksemmel@gmail.com";
in
{
  "spadesvision_gdrive_token.age".publicKeys = [ frederik ];
  "spadesvision_gdrive_client_secret.age".publicKeys = [ frederik ];
}
