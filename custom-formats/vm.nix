{ modulesPath }:
{

  imports = [ "${toString modulesPath}/virtualisation/qemu-vm.nix" ];

  formatAttr = "vm";

  disko.enableConfig = false;

  virtualisation = {
    memorySize = 1024 * 2;
    qemu = {
      options = [
        "-device virtio-gpu-gl-pci"
        "-display sdl,gl=on"
        "-vga none"
      ];
    };
  };
}
