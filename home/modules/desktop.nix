{
  flake,
  config,
  pkgs,
  lib,
  ...
}:
let
  cfg = config.profiles.desktop;
  wallpapers = pkgs.linkFarm "surf-wallpapers" (
    map (w: {
      name = "${w.id}.jpg";
      path = pkgs.fetchurl {
        inherit (w) url hash;
        name = "surf-${w.id}.jpg";
      };
    }) (import ../wallpapers.nix)
  );
  # Blurred, dimmed copies for niri's overview backdrop, drawn by a second awww
  # daemon behind the workspaces.
  wallpapersBlurred =
    pkgs.runCommand "surf-wallpapers-blurred" { nativeBuildInputs = [ pkgs.imagemagick ]; }
      ''
        mkdir $out
        for f in ${wallpapers}/*.jpg; do
          magick "$f" -resize 720x -blur 0x12 -filter Lanczos -resize 2880x -modulate 70 \
            -attenuate 0.06 +noise Gaussian -quality 90 "$out/$(basename "$f")"
        done
      '';
  wallpaperLink = "${config.xdg.stateHome}/wallpaper";
  # Shows a random wallpaper other than the current one and points
  # wallpaperLink at it, so the lock screen follows.
  wallpaperNext = pkgs.writeShellApplication {
    name = "wallpaper-next";
    runtimeInputs = [
      config.services.awww.package
      pkgs.coreutils
    ];
    text = ''
      current=$(readlink ${wallpaperLink} || true)
      mapfile -t candidates < <(printf '%s\n' ${wallpapers}/*.jpg | grep -vxF -- "$current" || true)
      next=$(printf '%s\n' "''${candidates[@]}" | shuf -n 1)
      awww img --transition-type fade --transition-duration 2 "$next"
      awww img --namespace backdrop --transition-type fade --transition-duration 2 "${wallpapersBlurred}/$(basename "$next")"
      ln -sfn "$next" ${wallpaperLink}
    '';
  };
  # Electron apps export CHROME_DESKTOP=<their desktop file>, and a Chromium
  # started from one of their links takes it as its Wayland app-id, so it
  # matched the floating bitwarden window rule.
  handlr-xdg-open = pkgs.writeShellScriptBin "xdg-open" ''
    unset CHROME_DESKTOP
    ${lib.getExe pkgs.handlr-regex} open "$@"
  '';
  # The --launcher-command of iwmenu, pwmenu and bzmenu; only iwmenu fills in
  # {password_flag:...}.
  yofiMenu = "yofi -p '{placeholder}' dialog";
  yofiIwmenu = "yofi -p '{placeholder}' {password_flag:--password} dialog";
  wlr-common-config = p: ''
    # Theming
    # A pixel size: wlr-which-key measures text at scale 1 but draws it hinted
    # at the output scale, and at 13px the drawn glyphs are no wider than the
    # measured ones, so the longest line keeps its right padding.
    font: ${p.font} 13px
    background: "${p.bg}ff"
    color: "${p.fg}"
    border: "${p.focus}ff"
    separator: '<span color="${p.focus}">  │  </span>'
    border_width: 2
    corner_r: 0
    padding: 18 # Defaults to corner_r
    # rows_per_column: 5 # No limit by default
    # column_padding: 25 # Defaults to padding

    # Anchor and margin
    anchor: center # One of center, left, right, top, bottom, bottom-left, top-left, etc.
    # Only relevant when anchor is not center
    margin_right: 0
    margin_bottom: 0
    margin_left: 0
    margin_top: 0

    # Permits key bindings that conflict with compositor key bindings.
    # Default is `false`.
    # inhibit_compositor_keyboard_shortcuts: true
  '';
  wlsunsetToggle = pkgs.writeShellApplication {
    name = "wlsunset-toggle";
    runtimeInputs = with pkgs; [
      libnotify
      systemd
    ];
    text = ''
      if systemctl --user is-active --quiet wlsunset; then
        systemctl --user stop wlsunset
        notify-send -a wlsunset -i weather-clear "Night light off"
      else
        systemctl --user start wlsunset
        notify-send -a wlsunset -i weather-clear-night "Night light on"
      fi
    '';
  };
  usbcShareToggle = pkgs.writeShellApplication {
    name = "usbc-share-toggle";
    runtimeInputs = with pkgs; [
      libnotify
      systemd
    ];
    text = ''
      if systemctl is-active --quiet usbc-share.service; then
        verb=stop
      else
        verb=start
      fi
      if out=$(/run/wrappers/bin/sudo -n /run/current-system/sw/bin/systemctl "$verb" usbc-share.service 2>&1); then
        if [ "$verb" = start ]; then
          notify-send -a usbc-share -i video-display "USB-C sharing on"
        else
          notify-send -a usbc-share -i video-display "USB-C sharing off"
        fi
      else
        notify-send -a usbc-share -i video-display -u critical "USB-C sharing" "$verb failed: $out"
      fi
    '';
  };
  airpodsToggle = pkgs.writeShellApplication {
    name = "airpods-toggle";
    runtimeInputs = with pkgs; [
      bluez
      gnugrep
      libnotify
    ];
    text = ''
      mac=34:0E:22:67:03:38
      connected() { bluetoothctl info "$mac" | grep -q "Connected: yes"; }

      if connected; then verb=disconnect; else verb=connect; fi
      id=$(notify-send -p -a bluetooth -i audio-headphones "AirPods" "''${verb^}ing…")
      out=$(bluetoothctl --timeout 15 "$verb" "$mac" 2>&1) || true
      if connected; then now=connect; else now=disconnect; fi
      if [ "$now" = "$verb" ]; then
        notify-send -r "$id" -a bluetooth -i audio-headphones "AirPods" "''${verb^}ed"
      else
        notify-send -r "$id" -a bluetooth -i audio-headphones -u critical "AirPods" "$verb failed: $(tail -n 1 <<<"$out")"
      fi
    '';
  };
  usbEjectAll = pkgs.writeShellApplication {
    name = "usb-eject-all";
    runtimeInputs = with pkgs; [
      coreutils
      gawk
      libnotify
      udisks
      util-linux
    ];
    text = ''
      eject() {
        local name fstype mountpoint
        while read -r name fstype mountpoint; do
          if [ -n "$mountpoint" ]; then
            udisksctl unmount --no-user-interaction -b "$name" || return 1
          fi
          if [ "$fstype" = crypto_LUKS ] && [ "$(lsblk -nro NAME "$name" | wc -l)" -gt 1 ]; then
            udisksctl lock --no-user-interaction -b "$name" || return 1
          fi
        done < <(lsblk -rnpo NAME,FSTYPE,MOUNTPOINT "$1" | tac)
        udisksctl power-off --no-user-interaction -b "$1"
      }

      mapfile -t disks < <(lsblk -dnbpo NAME,TRAN,SIZE | awk '$2 == "usb" && $3 > 0 { print $1 }')
      if [ ''${#disks[@]} -eq 0 ]; then
        notify-send -a usb-eject -i drive-removable-media "No USB drives"
        exit 0
      fi
      for disk in "''${disks[@]}"; do
        label=$(lsblk -dno MODEL "$disk" | xargs)
        label="''${label:-$disk}"
        if out=$(eject "$disk" 2>&1); then
          notify-send -a usb-eject -i drive-removable-media "Ejected $label"
        else
          notify-send -a usb-eject -i drive-removable-media -u critical "Eject $label failed" "$out"
        fi
      done
    '';
  };
  # TLP 1.8+ carries three named profiles (performance/balanced/power-saver,
  # configured through the _ON_AC/_ON_BAT/_ON_SAV suffixes of the host's
  # services.tlp.settings). The named verbs deliberately *clear* TLP's manual
  # mode -- only `tlp ac`/`tlp bat` latch it -- so a pick survives plugging and
  # unplugging by way of smart switching instead: TLP_AUTO_SWITCH=2 keeps the
  # current profile whenever it is not the power source's default. `tlp start`
  # and boot both hand the machine back to the assigned profile. The host grants
  # passwordless sudo for exactly these four invocations.
  tlpProfile = pkgs.writeShellApplication {
    name = "tlp-profile";
    runtimeInputs = [ pkgs.libnotify ];
    text = ''
      profile="$1"
      if out=$(/run/wrappers/bin/sudo -n /run/current-system/sw/bin/tlp "$profile" 2>&1); then
        if [ "$profile" = start ]; then
          notify-send -a tlp -i preferences-system-power "Power profile" "automatic"
        else
          notify-send -a tlp -i preferences-system-power "Power profile" "$profile"
        fi
      else
        notify-send -a tlp -i preferences-system-power -u critical "Power profile" "tlp $profile failed: $out"
      fi
    '';
  };
  # Two-way sync of the spadesvision shared drive with ~/spadesvision. Failures
  # and conflicts always notify. A run requested from the power menu finds the
  # id of its "Syncing" notification in spadesvisionSyncRequest and replaces it
  # with the result.
  spadesvisionDir = "${config.home.homeDirectory}/spadesvision";
  spadesvisionSyncRequest = "\"$XDG_RUNTIME_DIR\"/spadesvision-sync.notify-id";
  spadesvisionSync = pkgs.writeShellApplication {
    name = "spadesvision-sync";
    runtimeInputs = with pkgs; [
      config.programs.rclone.package
      coreutils
      findutils
      getent
      gnugrep
      gnused
      libnotify
    ];
    text = ''
      notify() {
        local id=""
        if [ -f ${spadesvisionSyncRequest} ]; then
          id=$(cat ${spadesvisionSyncRequest})
          rm -f ${spadesvisionSyncRequest}
        fi
        notify-send -a rclone -i folder-remote ''${id:+-r "$id"} "$@"
      }

      if ! getent hosts www.googleapis.com >/dev/null; then
        [ -f ${spadesvisionSyncRequest} ] && notify "Spadesvision sync skipped" "Offline"
        exit 0
      fi

      log=$(mktemp)
      started=$(mktemp)
      trap 'rm -f "$log" "$started"' EXIT

      if ! rclone bisync spadesvision:/ ${spadesvisionDir} \
        --filter-from ${config.home.homeDirectory}/.rclone_filter_spadesvision.txt \
        --fast-list --create-empty-src-dirs \
        --compare size,modtime,checksum --slow-hash-sync-only \
        --drive-skip-gdocs --drive-chunk-size 64Mi \
        --resilient --recover --max-lock 15m \
        --transfers 8 --checkers 16 \
        --color NEVER -v 2>&1 | tee "$log"; then
        notify -u critical "Spadesvision sync failed" \
          "$(grep -E 'ERROR|CRITICAL' "$log" | tail -n 3 | sed -E 's/^.*(ERROR|CRITICAL) *: *//')"
        exit 1
      fi

      conflicts=$(find ${spadesvisionDir} -cnewer "$started" -name '*.conflict[0-9]*' -printf '%P\n')
      if [ -n "$conflicts" ]; then
        notify -u critical "Spadesvision sync: conflicts" "$conflicts"
      elif [ -f ${spadesvisionSyncRequest} ]; then
        summary=$(sed -nE \
          -e 's/.*Path1: +[0-9]+ changes: +/From Drive: /p' \
          -e 's/.*Path2: +[0-9]+ changes: +/From local: /p' "$log" | tr -s ' ')
        notify "Spadesvision synced" "''${summary:-No changes}"
      fi
    '';
  };
  spadesvisionSyncNow = pkgs.writeShellApplication {
    name = "spadesvision-sync-now";
    runtimeInputs = with pkgs; [
      libnotify
      systemd
    ];
    text = ''
      notify-send -p -a rclone -i folder-remote "Spadesvision sync" "Syncing…" > ${spadesvisionSyncRequest}
      systemctl --user start --no-block spadesvision-sync.service
    '';
  };
  # The current mode's colours as shell variables. The bar scripts rerun every
  # few seconds, so they pick up a switch on their next run.
  shellPalette =
    names:
    let
      assign = p: lib.concatMapStringsSep " " (n: "${n}='${p.${n}}'") names;
    in
    ''
      if [ "$(cat ${config.theme.dir}/mode 2>/dev/null)" = light ]; then
        ${assign config.theme.palettes.light}
      else
        ${assign config.theme.palettes.dark}
      fi
    '';
  # Bar cell for the machine: CPU, memory and disk, coloured once they cross a
  # threshold, plus the TLP profile when it is not the automatic one.
  # `tlp-stat -m` needs no root: it reads /run/tlp/last_pwr, which tlp leaves
  # world-readable, and prints "<profile>/<CODE>"; the profile shows only when
  # its code differs from the power source's own (AC on mains, BAT on battery).
  barSystem = pkgs.writeShellApplication {
    name = "bar-system";
    runtimeInputs = with pkgs; [
      coreutils
      gawk
    ];
    text = ''
      ${shellPalette [
        "muted"
        "fg"
        "yellow"
        "red"
      ]}
      colors=("$fg" "$yellow" "$red")
      cells=()
      cell() {
        cells+=("<span color=\"$muted\">$1</span> <span color=\"''${colors[$3]}\">$2</span>")
      }
      level() {
        if (($1 >= $3)); then echo 2; elif (($1 >= $2)); then echo 1; else echo 0; fi
      }

      read -r _ a b c i1 _ < /proc/stat
      t1=$((a + b + c + i1))
      sleep 0.5
      read -r _ a b c i2 _ < /proc/stat
      t2=$((a + b + c + i2))
      cpu=$((100 * ((t2 - t1) - (i2 - i1)) / (t2 - t1 > 0 ? t2 - t1 : 1)))
      l=$(level "$cpu" 70 90)
      cell CPU "$(printf '%-4s' "$cpu%")" "$l"

      read -r total avail < <(awk '/^MemTotal/ { t = $2 } /^MemAvailable/ { a = $2 } END { print t, a }' /proc/meminfo)
      l=$(level $((100 * (total - avail) / total)) 75 90)
      cell MEM "$(awk -v k=$((total - avail)) 'BEGIN { printf "%-5s", sprintf("%.1fG", k / 1048576) }')" "$l"

      dsk=$(df --output=pcent / | tail -1 | tr -dc 0-9)
      l=$(level "$dsk" 85 95)
      cell DSK "$(printf '%-4s' "$dsk%")" "$l"

      mode=$(/run/current-system/sw/bin/tlp-stat -m) || mode="unknown/???"
      profile=''${mode%%/*}
      code=''${mode#*/}
      code=''${code%% *}
      source_code=BAT
      [ "$(cat /sys/class/power_supply/AC0/online)" = 1 ] && source_code=AC
      if [ "$code" != "$source_code" ]; then
        l=0
        [ "$code" = SAV ] && l=1
        cell PWR "$profile" "$l"
      fi

      printf '%s' "''${cells[0]}"
      printf '  %s' "''${cells[@]:1}"
      echo
    '';
  };
  barNet = pkgs.writeShellApplication {
    name = "bar-net";
    runtimeInputs = with pkgs; [
      gawk
      gnused
      iwd
    ];
    text = ''
      ssid=$(iwctl station wlan0 show 2>/dev/null \
        | sed 's/\x1b\[[0-9;]*m//g' \
        | awk '/Connected network/ { $1 = $2 = ""; sub(/^ +/, ""); sub(/ +$/, ""); print }')
      ${shellPalette [ "muted" ]}
      echo "<span color='$muted'>NET</span> ''${ssid:-offline}"
    '';
  };
  # claudebar speaks waybar's JSON; its text already carries pango colours.
  barClaude = pkgs.writeShellApplication {
    name = "bar-claude";
    runtimeInputs = [
      pkgs.claudebar
      pkgs.jq
    ];
    text = ''
      ${shellPalette [
        "green"
        "yellow"
        "orange"
        "red"
      ]}
      out=$(claudebar --format 'Claudio {session_pct}% · {session_reset}' \
        --color-low "$green" --color-mid "$yellow" --color-high "$orange" --color-critical "$red")
      if [ "''${1:-}" = tooltip ]; then
        # ironbar sets tooltips as plain text, not pango markup.
        jq -r '.tooltip | gsub("<[^>]*>"; "") | gsub("&lt;"; "<") | gsub("&gt;"; ">") | gsub("&amp;"; "&")' <<<"$out"
      else
        jq -r .text <<<"$out"
      fi
    '';
  };
  ironbarCss = p: ''
    * {
      font-family: "${p.font}", monospace;
      font-size: 12px;
      border: none;
      border-radius: 0;
      box-shadow: none;
      text-shadow: none;
      min-height: 0;
    }

    .background,
    #bar {
      background: ${p.bg};
      color: ${p.fg};
    }

    button {
      background: none;
      color: ${p.fg};
      padding: 0;
      margin: 0;
    }

    .widget {
      padding: 0 7px;
    }

    .cell {
      margin-left: 11px;
      padding-left: 18px;
      border-left: 1px solid ${p.focus};
    }

    .workspaces {
      padding: 0;
    }

    .workspaces .item {
      padding: 0 7px;
    }

    .workspaces .item:hover {
      background: ${p.bgAlt};
    }

    .workspaces .item.focused {
      background: ${p.focus};
    }

    .workspaces .item.urgent {
      background: ${p.red};
    }

    .focused .label {
      color: ${p.muted};
    }

    .clock {
      font-weight: bold;
    }

    .battery.warning {
      color: ${p.yellow};
    }

    .battery.critical {
      color: ${p.red};
    }

    .popup {
      background: ${p.bg};
      border: 2px solid ${p.focus};
      padding: 10px;
      color: ${p.fg};
    }
  '';
  vicinaeTheme = p: {
    meta = {
      version = 1;
      name = if p.name == "dark" then "Terminal" else "Terminal Light";
      description = "Flat, like the bar";
      variant = p.name;
      inherits = "vicinae-${p.name}";
    };
    colors = {
      core = {
        background = p.bg;
        foreground = p.fg;
        secondary_background = p.bg;
        border = p.border;
        accent = p.yellow;
        accent_foreground = p.bg;
      };
      main_window.border = p.focus;
      settings_window.border = p.focus;
      accents = {
        inherit (p)
          blue
          green
          magenta
          orange
          red
          yellow
          cyan
          ;
        purple = p.magenta;
      };
      text = {
        inherit (p) muted;
        placeholder = p.faint;
        danger = p.red;
        success = p.green;
        selection = {
          background = p.focus;
          foreground = p.fg;
        };
      };
      input = {
        inherit (p) border;
        border_focus = p.focus;
        border_error = p.red;
      };
      button.primary = {
        background = p.bgAlt;
        foreground = p.fg;
      };
      list.item = {
        hover = {
          foreground = p.fg;
          secondary_foreground = p.muted;
        };
        selection = {
          background = p.border;
          foreground = p.fg;
          secondary_background = p.border;
          secondary_foreground = p.muted;
        };
      };
      grid.item.background = p.surface;
      scrollbars.background = p.border;
      loading = {
        bar = p.yellow;
        spinner = p.fg;
      };
    };
  };
  rerun-icon = pkgs.callPackage ../../packages/rerun-icon.nix { };
  # Route audio over HDMI when the tv kanshi profile is applied. The card's
  # analog profile has a higher priority than the HDMI one, so wireplumber
  # never picks HDMI on its own.
  tvAudio = pkgs.writeShellApplication {
    name = "tv-audio";
    runtimeInputs = with pkgs; [
      jq
      pipewire
      wireplumber
    ];
    text = ''
      card="${cfg.hdmiAudio.card}"
      profile="output:hdmi-stereo+input:analog-stereo"
      sink="alsa_output.${cfg.hdmiAudio.device}.hdmi-stereo"

      card_id=$(pw-dump | jq -r --arg n "$card" \
        'first(.[] | select(.info.props."device.name" == $n) | .id)')
      profile_index=$(pw-dump "$card_id" | jq -r --arg p "$profile" \
        'first(.[0].info.params.EnumProfile[] | select(.name == $p) | .index)')
      wpctl set-profile "$card_id" "$profile_index"

      # The sink node only shows up once the profile switch has settled.
      sink_id=""
      for _ in $(seq 20); do
        sink_id=$(pw-dump | jq -r --arg n "$sink" \
          'first(.[] | select(.info.props."node.name" == $n) | .id) // empty')
        if [ -n "$sink_id" ]; then
          break
        fi
        sleep 0.25
      done
      wpctl set-default "$sink_id"
    '';
  };
in
{
  imports = with flake.inputs; [
    veila.homeModules.default
    # centerpiece.hmModules."x86_64-linux".default
  ];

  options.profiles.desktop = {
    enable = lib.mkEnableOption "graphical desktop (niri, ironbar, GUI apps)";
    wallpaper = lib.mkOption {
      type = lib.types.str;
      default = wallpaperLink;
      description = "Wallpaper image used by the lock screen and other desktop services.";
    };
    hdmiAudio = {
      device = lib.mkOption {
        type = lib.types.str;
        default = "pci-0000_00_1f.3";
        description = "ALSA device id of the codec carrying HDMI output.";
      };
      card = lib.mkOption {
        type = lib.types.str;
        readOnly = true;
        default = "alsa_card.${cfg.hdmiAudio.device}";
        description = "Pipewire device.name of that codec, read by the host to target its profile-set.";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    theme.enable = true;
    theme.onSwitch = ''
      cat "$files/ironbar.css" > ${config.theme.dir}/ironbar.css
      live ${config.services.mako.package}/bin/makoctl reload
      live ${lib.getExe config.programs.niri.package} msg action load-config-file
      live ${lib.getExe pkgs.dconf} write /org/gnome/desktop/interface/color-scheme "'prefer-$mode'"
      live ${lib.getExe pkgs.dconf} write /org/gnome/desktop/interface/gtk-theme \
        "'$([ "$mode" = dark ] && echo Adwaita-dark || echo Adwaita)'"
      live ${lib.getExe pkgs.dconf} write /org/gnome/desktop/interface/monospace-font-name "'$family 11'"
      # vicinae does not notice `current` moving under its import.
      [ "$font" = "$previous_font" ] || live ${pkgs.systemd}/bin/systemctl --user try-restart vicinae.service
    '';

    home.packages = with pkgs; [
      (lib.hiPrio handlr-xdg-open)
      wallpaperNext
      ironbar
      yofi
      gnome-themes-extra # Adwaita-dark for GTK 3

      # Networking & Communication
      transmission_4-gtk
      iwmenu
      pwmenu
      bzmenu
      mqttx

      # File & Data Management
      # nautilus (systemPackages in fstux) is the one GUI file manager: it owns
      # org.freedesktop.FileManager1, which chromium's "show in files" calls.
      pcloud
      handlr-regex

      # Audio & Video
      # pavucontrol
      spotify
      shotcut
      wf-recorder
      pwvucontrol

      # Productivity & Document Tools
      zotero
      xournalpp
      # libreoffice
      gridcraft
      wordcraft
      # ungoogled-chromium
      evince
      # papers issues with saving and icons
      signal-desktop
      sqlitebrowser
      inlyne

      # Desktop / Wayland utilities
      overskride
      rerun
      pass-wayland
      wl-clipboard
      xdg-utils
      xwayland-satellite
      wlr-which-key
      brightnessctl
      claudebar

      # Remote desktop
      rustdesk-flutter
      anydesk
    ];

    # One look for both file-picker code paths: xdg-desktop-portal-gtk and apps
    # that call gtk_file_chooser_dialog_new themselves (onlyoffice). Leaving
    # gtk3.bookmarks unset keeps ~/.config/gtk-3.0/bookmarks writable and shared.
    gtk = {
      enable = true;
      # Not Cantarell or Adwaita Sans: fontconfig 2.18 tags both with the
      # system-ui generic, chromium's own lookup then rejects them and its UI
      # falls back to an unrelated font (NixOS/nixpkgs#541553). fc-match is
      # unaffected. Inter is what Adwaita Sans derives from and survives it.
      font = {
        name = "Inter";
        size = 11;
      };
      # No theme or colorScheme: both would pin dconf keys on every activation.
      # The dark/light switch writes gtk-theme and color-scheme instead.
      iconTheme = {
        name = "Adwaita";
        package = pkgs.adwaita-icon-theme;
      };
      # libadwaita (nautilus) only honours color-scheme; post-26.05 default.
      gtk4.theme = null;
    };

    home.pointerCursor = {
      enable = true;
      name = "Adwaita";
      package = pkgs.adwaita-icon-theme;
      size = 24;
      gtk.enable = true;
    };

    xdg.mimeApps = {
      enable = true;
      defaultApplications =
        let
          browser = "chromium-browser.desktop";
          spreadsheet = "ai.storyteller.gridcraft.desktop";
          document = "ai.storyteller.wordcraft.desktop";
        in
        {
          "x-scheme-handler/http" = browser;
          "x-scheme-handler/https" = browser;
          "x-scheme-handler/about" = browser;
          "x-scheme-handler/unknown" = browser;
          "text/html" = browser;
          "application/xhtml+xml" = browser;
          "x-scheme-handler/claude-cli" = "claude-code-url-handler.desktop";
          "inode/directory" = "org.gnome.Nautilus.desktop";
          "application/pdf" = "org.pwmt.zathura.desktop";
          "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" = spreadsheet;
          "application/vnd.ms-excel.sheet.macroEnabled.12" = spreadsheet;
          "application/vnd.ms-excel.sheet.binary.macroEnabled.12" = spreadsheet;
          "application/vnd.ms-excel" = spreadsheet;
          "application/vnd.oasis.opendocument.spreadsheet" = spreadsheet;
          "application/vnd.openxmlformats-officedocument.wordprocessingml.document" = document;
          "application/vnd.oasis.opendocument.text" = document;
          "application/rtf" = document;
          "image/*" = "imv.desktop";
          "video/*" = "mpv.desktop";
          "audio/*" = "mpv.desktop";
        };
    };
    xdg.configFile."mimeapps.list".force = true;

    home.file = {
      ".rclone_filter_spadesvision.txt".text = ''
        - Marketing/Material für Swisscasino Video/**
        - Development/Jetson Server Dev/*Releases*/**
      '';
      ".config/handlr/handlr.toml".text = ''
        enable_selector = true
        selector = "${lib.getExe pkgs.yofi} dialog"
        term_exec_args = '-e'
      '';
      ".config/inlyne/inlyne.toml".source =
        config.lib.file.mkOutOfStoreSymlink "${config.theme.dir}/current/inlyne.toml";
    };
    theme.files."inlyne.toml" = p: ''
      theme = "Dark"
      page-width = 1150
      lines-to-scroll = 3.0

      [font-options]
      regular-font = "Inter"
      monospace-font = "${p.font}"

      [dark-theme]
      code-highlighter = "monokai-extended"
    '';

    theme.files."which-key-power.yaml" = p: ''
      ${wlr-common-config p}
      menu:
        "w":
          desc: Wifi Menu
          cmd: iwmenu -l custom --launcher-command "${yofiIwmenu}"
        "a":
          desc: Audio Menu
          cmd: pwmenu -l custom --launcher-command "${yofiMenu}" -m output-devices
        "b":
          desc: Bluetooth Menu
          cmd: bzmenu -l custom --launcher-command "${yofiMenu}"
        "h":
          desc: Toggle AirPods
          cmd: ${lib.getExe airpodsToggle}
        "n":
          desc: Toggle Night Light
          cmd: ${lib.getExe wlsunsetToggle}
        "t":
          desc: Toggle USB-C Internet Sharing
          cmd: ${lib.getExe usbcShareToggle}
        "e":
          desc: Eject USB Drives
          cmd: ${lib.getExe usbEjectAll}
        "p":
          desc: Next Wallpaper
          cmd: systemctl --user start wallpaper-next.service
        "g":
          desc: Sync Spadesvision Drive
          cmd: ${lib.getExe spadesvisionSyncNow}
        "c":
          desc: CPU Profile
          submenu:
            "p":
              desc: Performance
              cmd: ${lib.getExe tlpProfile} performance
            "b":
              desc: Balanced
              cmd: ${lib.getExe tlpProfile} balanced
            "s":
              desc: Power Saver (no turbo, half clock)
              cmd: ${lib.getExe tlpProfile} power-saver
            "a":
              desc: Automatic (follow power source)
              cmd: ${lib.getExe tlpProfile} start
        "s":
          desc: Sleep
          cmd: systemctl suspend
        "r":
          desc: Reboot
          cmd: reboot
        "o":
          desc: Off
          cmd: poweroff
        "l":
          desc: Lock
          cmd: ${config.programs.veila.package}/bin/veila lock
        "i":
          desc: Inhibit Sleep (5h)
          cmd: systemd-inhibit --what=handle-lid-switch sleep 5h
        "u":
          desc: Uninhibit Sleep
          cmd: pkill -x systemd-inhibit
        "d":
          desc: Dark / Light
          cmd: darkman toggle
        "f":
          desc: Font
          submenu:
            "j":
              desc: JetBrains Mono
              cmd: theme-font jetbrains
            "n":
              desc: Nordwand Mono
              cmd: theme-font nordwand
    '';
    theme.files."which-key-opener.yaml" = p: ''
      ${wlr-common-config p}
      menu:
        "w":
          desc: Browser
          cmd: handlr launch text/html
        "f":
          desc: Files
          cmd: handlr launch inode/directory
        "l":
          desc: Linear
          cmd: chromium --app=https://linear.app
        "r":
          desc: Rerun
          cmd: rerun
        "m":
          desc: MQTTX
          cmd: mqttx
        "b":
          desc: Bitwarden
          cmd: rbw-menu
        "a":
          desc: Audio Mixer
          cmd: pwvucontrol
        "v":
          desc: Resume Video
          cmd: mpv --idle=once --force-window --input-commands='script-binding memo-last'
        "h":
          desc: Video History
          cmd: mpv --idle=once --force-window --input-commands='script-binding memo-history'
    '';

    # Icon= is a theme name, and nixpkgs' rerun installs no icon at all, so the
    # lookup failed and launchers drew their fallback glyph. recursive, because
    # ~/.local/share/icons/hicolor also holds unmanaged icons (pcloud's
    # AppImage) that a whole-directory symlink would displace.
    xdg.dataFile."icons/hicolor" = {
      source = "${rerun-icon}/share/icons/hicolor";
      recursive = true;
    };

    # Define an XDG desktop entry for Rerun
    xdg.desktopEntries = {
      rerun = {
        name = "Rerun";
        genericName = "Multimodal Data Viewer";
        comment = "Visualize streams of multimodal data";
        exec = "rerun %F";
        terminal = false;
        icon = "rerun";
        categories = [
          "Development"
          "Science"
          "Utility"
        ];
      };
      linear = {
        name = "Linear";
        genericName = "Issue Tracker";
        exec = "chromium --app=https://linear.app";
        terminal = false;
        icon = pkgs.fetchurl {
          url = "https://linear.app/static/apple-touch-icon.png";
          hash = "sha256-PSLVOQeCAkXsFySklU0+GAtw8hslUml5hrPxQaZzDvc=";
        };
        categories = [
          "Development"
          "Office"
        ];
      };
    };

    # NOTE this was an attempt to make ly work. But somehow graphical-session.target starts way to early
    # systemd.user.services.waybar.Unit.Requisite = [ "graphical-session.target" ];
    # systemd.user.services.swayidle.Unit.Requisite = [ "graphical-session.target" ];

    services.kanshi = {
      enable = true;
      settings = [
        {
          output.criteria = "eDP-1";
          output.mode = "2880x1800@90Hz";
          output.scale = 1.75;
        }
        {
          profile.name = "laptop";
          profile.outputs = [
            {
              criteria = "eDP-1";
            }
          ];
        }
        {
          profile.name = "mobile";
          profile.outputs = [
            {
              criteria = "eDP-1";
              position = "1494,0";
            }
            {
              criteria = "Lenovo Group Limited M14d *";
              position = "0,0";
            }
          ];
        }
        {
          # TV above the laptop, horizontally centered on it:
          # (3840 - 2880/1.75) / 2 = 1097
          profile.name = "tv";
          profile.outputs = [
            {
              criteria = "Samsung Electric Company SAMSUNG *";
              mode = "3840x2160@24Hz";
              scale = 1.0;
              position = "0,0";
            }
            {
              criteria = "eDP-1";
              position = "1097,2160";
            }
          ];
          profile.exec = [ (lib.getExe tvAudio) ];
        }
        {
          profile.name = "office";
          profile.outputs = [
            {
              criteria = "eDP-1";
              position = "202,1153";
            }
            {
              criteria = "DP-3";
              scale = 1.25;
              position = "0,0";
            }
          ];
        }
      ];
    };
    # Locking is handled by veila's own idle monitor, which takes a logind delay
    # inhibitor so the lock surface is up before the system actually suspends.
    #
    # Blanking, however, cannot be left to veila: it drives DPMS through
    # zwlr_output_power_management_v1, which niri does not implement (see the
    # note on lock.screen_off_seconds below). So we blank compositor-side
    # instead, reproducing what veila's screen_off_seconds=60 was meant to do:
    # the display goes dark 60s after veila locks.
    # No resumeCommand is needed -- niri calls activate_monitors() itself on the
    # next input event -- but it is harmless, idempotent, and makes the intent
    # explicit rather than relying on that behaviour staying true.
    services.swayidle = {
      enable = true;
      timeouts = [
        {
          timeout = config.programs.veila.idle.lockAfter + 60;
          command = "${config.programs.niri.package}/bin/niri msg action power-off-monitors";
          resumeCommand = "${config.programs.niri.package}/bin/niri msg action power-on-monitors";
        }
      ];
    };
    services.mako = {
      enable = true;
      # Terminal look: monospace, black, square, and the grey of niri's focus
      # ring; stacked top-right. Papirus only supplies
      # notification icons (it has light variants for dark backgrounds); GTK
      # keeps Adwaita. hicolor comes last so apps' own icons still resolve.
      # All of it lives in the theme, since criteria sections can't be split
      # around an include.
      extraConfig = "include=${config.theme.dir}/current/mako";
    };
    theme.files.mako =
      p:
      let
        papirus = "${pkgs.papirus-icon-theme}/share/icons";
        icons =
          if p.name == "dark" then
            "${papirus}/Papirus-Dark:${papirus}/Papirus"
          else
            "${papirus}/Papirus:${papirus}/Papirus-Dark";
      in
      ''
        font=${p.font} 9
        anchor=top-right
        background-color=${p.bg}
        text-color=${p.fg}
        border-size=2
        border-color=${p.focus}
        border-radius=0
        padding=10,12
        margin=6
        width=380
        icon-path=${icons}:${config.home.profileDirectory}/share/icons/hicolor:/run/current-system/sw/share/icons/hicolor
        max-icon-size=32
        format=<span foreground="${p.muted}">%a</span>\n<b>%s</b>\n%b
        default-timeout=10000

        [urgency=low]
        text-color=${p.muted}

        [urgency=critical]
        border-color=${p.red}
        default-timeout=0

        [app-name=Spotify]
        default-timeout=3000

        [app-name=Claudio]
        border-color=${p.green}
        format=<b>%s</b>\n%b

        [app-name=Claudio urgency=normal]
        border-color=${p.yellow}

        [app-name=Claudio urgency=critical]
        border-color=${p.orange}
      '';
    services.awww.enable = true;

    # Repoints the link when it is missing or targets a wallpaper set from an
    # older generation.
    home.activation.wallpaperLink = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
      case "$(readlink ${wallpaperLink} || true)" in
        ${wallpapers}/*) ;;
        *) run mkdir -p ${config.xdg.stateHome} && run ln -sfn ${wallpapers}/${(builtins.head (import ../wallpapers.nix)).id}.jpg ${wallpaperLink} ;;
      esac
    '';

    systemd.user.services.awww-backdrop = {
      Unit = {
        Description = "awww-daemon for the overview backdrop";
        PartOf = [ "graphical-session.target" ];
        After = [ "graphical-session.target" ];
        ConditionEnvironment = "WAYLAND_DISPLAY";
      };
      Service = {
        ExecStart = "${config.services.awww.package}/bin/awww-daemon --namespace backdrop";
        Restart = "always";
        RestartSec = 10;
      };
      Install.WantedBy = [ "graphical-session.target" ];
    };

    systemd.user.services.wallpaper-next = {
      Unit = {
        Description = "Switch to the next surf wallpaper";
        After = [
          "awww.service"
          "awww-backdrop.service"
        ];
        Requires = [
          "awww.service"
          "awww-backdrop.service"
        ];
        ConditionEnvironment = "WAYLAND_DISPLAY";
      };
      Service = {
        Type = "oneshot";
        ExecStart = lib.getExe wallpaperNext;
      };
    };
    systemd.user.timers.wallpaper-next = {
      Unit = {
        Description = "Rotate the surf wallpaper every 30 minutes";
        PartOf = [ "graphical-session.target" ];
      };
      Timer.OnCalendar = "*:0/30";
      Install.WantedBy = [ "graphical-session.target" ];
    };
    systemd.user.services.spadesvision-sync = {
      Unit = {
        Description = "Bisync the spadesvision shared drive with ~/spadesvision";
        After = [
          "agenix.service"
          "rclone-config.service"
        ];
        Wants = [ "rclone-config.service" ];
      };
      Service = {
        Type = "oneshot";
        ExecStart = lib.getExe spadesvisionSync;
        Nice = 10;
        IOSchedulingClass = "idle";
      };
    };
    systemd.user.timers.spadesvision-sync = {
      Unit = {
        Description = "Bisync the spadesvision shared drive every hour";
        PartOf = [ "graphical-session.target" ];
      };
      Timer = {
        OnCalendar = "hourly";
        Persistent = true;
      };
      Install.WantedBy = [ "graphical-session.target" ];
    };
    services.wlsunset = {
      enable = true;
      sunrise = "07:00";
      sunset = "19:00";
      temperature.night = 3000;
    };

    programs.zed-editor.enable = true;

    # Lock screen. NOTE: PAM cannot be set up from home-manager, so
    # `security.pam.services.veila` lives in the host config.
    programs.veila = {
      enable = true;
      service.enable = true;
      idle = {
        enable = true;
        lockAfter = 600;
        lockBeforeSleep = true;
      };
      settings = {
        theme = "normandy";

        # Everything below overrides the normandy theme.
        background = {
          mode = "file";
          path = cfg.wallpaper;
          scaling = "fill";
        };

        lock = {
          hide_cursor = true;
          # MUST stay 0 on niri. Veila blanks via zwlr_output_power_management_v1,
          # which niri does not implement, so this never worked -- but worse, the
          # failure pegs a CPU core: once the delay expires, ScreenOffState::due_in
          # returns Duration::ZERO forever (the power-off request silently no-ops,
          # so outputs_powered_off is never set), and that zero becomes the
          # curtain's calloop timeout -- a non-blocking dispatch in a tight loop.
          # Measured: 105% CPU and 95 C package temp for an entire lock.
          # To actually blank, drive niri's `power-off-monitors` from an idle
          # timeout (swayidle) rather than asking veila to do it.
          screen_off_seconds = 0;
        };

        # Widgets. Positions/colors come from normandy.
        battery.enabled = true;
        weather = {
          enabled = true;
          location = "Zurich";
          latitude = 47.37;
          longitude = 8.5417;
          unit = "celsius";
        };
        visuals = {
          # Which of us/us(intl)/us(colemak) is active — typing the password in
          # the wrong layout is otherwise silent.
          keyboard.enabled = true;
          # Caps is remapped to Ctrl and its LED shows the xkb group, so the
          # caps indicator would only ever be noise.
          caps_lock.enabled = false;
          avatar.enabled = false;
          username.enabled = false;
          now_playing.enabled = true;
          eye.enabled = false;
        };
      };
    };

    # The veila daemon spawns veila-curtain as a plain child, so the curtain inherits the
    # daemon's environment rather than the session's. The veila module orders
    # veila.service after graphical-session.target only, which is not a barrier for
    # WAYLAND_DISPLAY: niri publishes it with `systemctl --user import-environment`
    # once its socket is up, and the manager hands each service the environment
    # block as it was at spawn time. Start veila before niri -- which happens
    # whenever graphical-session.target is reached outside niri's own transaction,
    # e.g. after niri is OOM-killed and the session restarts -- and it holds an
    # environment with no WAYLAND_DISPLAY for its whole life, so every curtain
    # dies with "Could not find wayland compositor" and locking silently fails.
    # BindsTo pulls niri.service into veila's start transaction so After can
    # actually order against it.
    systemd.user.services.veila.Unit = {
      After = [ "niri.service" ];
      BindsTo = [ "niri.service" ];
    };

    programs.sherlock = {
      enable = false;
      settings = {
        aliases = { };
        config = {
        };
        launchers = [
          {
            name = "App Launcher";
            type = "app_launcher";
            args = { };
            priority = 1;
            home = true;
          }
        ];
        style = null;
      };
    };

    programs.zathura = {
      enable = true;
      extraConfig = ''
        set selection-clipboard clipboard
      '';
    };

    programs.onlyoffice = {
      enable = true;
    };

    xdg.configFile."yofi/yofi.config".source =
      config.lib.file.mkOutOfStoreSymlink "${config.theme.dir}/current/yofi.toml";
    theme.files."yofi.toml" = p: ''
      width = 560
      height = 380
      font = "${p.font}"
      font_size = 15
      font_color = "${p.fg}"
      bg_color = "${p.bg}"
      bg_border_color = "${p.focus}"
      bg_border_width = 2.0
      corner_radius = "0"

      [input_text]
      font_color = "${p.fg}"
      prompt_color = "${p.faint}"
      bg_color = "${p.bgAlt}"
      corner_radius = "0"
      margin = "12"
      padding = "6 8"

      [list_items]
      font_color = "${p.muted}"
      selected_font_color = "${p.fg}"
      match_color = "${p.yellow}"
      margin = "8 18"
      hide_actions = true
    '';

    programs.chromium = {
      enable = true;
      commandLineArgs = [ "--enable-features=TouchpadOverscrollHistoryNavigation" ];
      package = (pkgs.chromium.override { enableWideVine = true; });
      extensions = [
        { id = "nngceckbapebfimnlniiiahkandclblb"; } # bitwarden
      ];
    };
    # programs.centerpiece = {
    #   enable = true;
    #   config = {
    #     color = {
    #       text = "#ffffff";
    #       background = "#000000";
    #     };
    #     plugin = {
    #       applications = {
    #         enable = true;
    #       };
    #       brave_bookmarks = {
    #         enable = false;
    #       };
    #       brave_history = {
    #         enable = false;
    #       };
    #       brave_progressive_web_apps = {
    #         enable = false;
    #       };
    #       clock = {
    #         enable = true;
    #       };
    #       firefox_bookmarks = {
    #         enable = false;
    #       };
    #       firefox_history = {
    #         enable = true;
    #       };
    #       git_repositories = {
    #         enable = false;
    #       };
    #       gitmoji = {
    #         enable = true;
    #       };
    #       resource_monitor_battery = {
    #         enable = true;
    #       };
    #       resource_monitor_cpu = {
    #         enable = true;
    #       };
    #       resource_monitor_disks = {
    #         enable = true;
    #       };
    #       resource_monitor_memory = {
    #         enable = true;
    #       };
    #       niri_windows = {
    #         enable = true;
    #       };
    #       system = {
    #         enable = true;
    #       };
    #       wifi = {
    #         enable = true;
    #       };
    #     };
    #   };

    # enables a systemd service to index git-repositories
    #   services.index-git-repositories = {
    #     enable = true;
    #     interval = "5min";
    #   };
    # };

    programs.niri.package = lib.mkDefault pkgs.niri;

    # settings.includes would land at the top, where the settings after it
    # override the colours. At option-default priority this merges with the
    # rendered settings and follows them.
    programs.niri.config = lib.mkOptionDefault [
      {
        name = "include";
        arguments = [ "${config.theme.dir}/current/niri.kdl" ];
        properties.optional = true;
      }
    ];
    # background-color shows until awww draws the wallpaper.
    theme.files."niri.kdl" = p: ''
      layout {
          background-color "${p.bg}"
          focus-ring {
              active-color "${p.focus}"
              inactive-color "${p.bg}"
          }
      }
      overview {
          backdrop-color "${p.bg}"
      }
    '';

    programs.niri.settings = {
      "prefer-no-csd" = true;
      layer-rules = [
        {
          matches = [ { namespace = "^awww-daemonbackdrop$"; } ];
          place-within-backdrop = true;
        }
      ];
      cursor = {
        inherit (config.home.pointerCursor) size;
        theme = config.home.pointerCursor.name;
      };
      ### Inputs
      input.focus-follows-mouse.enable = true;
      input.touchpad = {
        accel-profile = "adaptive";
        accel-speed = 0.3;
        scroll-factor = 0.5;
        dwt = true;
      };
      input.keyboard = {
        repeat-delay = 250;
        repeat-rate = 30;
        xkb.layout = "us, us(intl), us(colemak)";
        xkb.options = "ctrl:nocaps, compose:ralt, grp:win_space_toggle, grp_led:caps, altwin:swap_alt_win";
      };

      ## Outputs
      outputs."eDP-1" = {
        # scale = 2.0;
        position = {
          x = 0;
          y = 0;
        };
      };
      outputs."Dell Inc. DELL U2721DE B0GC623" = {
        scale = 1.25;
        position = {
          x = 0;
          y = -1152 - 1;
        };
      };
      outputs."Samsung Electric Company SAMSUNG 0x01000E00" = {

      };
      # outputs."HDMI-A-1" = {
      #   mode = {
      #     width = 3840;
      #     height = 2160;
      #     refresh = 23.976;
      #   };
      # };

      ### Startup
      spawn-at-startup = [
        {
          command = [
            (lib.getExe pkgs.xwayland-satellite)
          ];
        }
      ];
      environment = {
        QT_QPA_PLATFORM = "wayland";
      };
      hotkey-overlay.skip-at-startup = true;

      ### Visuals
      layout.gaps = 0; # disables the default 16px gap :contentReference[oaicite:0]{index=0}
      layout.preset-column-widths = map (p: { proportion = p; }) [
        0.25
        0.5
        0.75
      ];
      layout.preset-window-heights = map (p: { proportion = p; }) [
        0.25
        0.5
        0.75
      ];
      # layout.border = {
      #   enable = true;
      #   width = 2;
      #   inactive.color = "#000000";
      #   active.color = "#505050";
      # };
      # The dark colours hold until niri.kdl from the theme, included last,
      # overrides them; without them niri falls back to its blue.
      layout.background-color = config.theme.palettes.dark.bg;
      overview.backdrop-color = config.theme.palettes.dark.bg;
      layout.focus-ring = {
        enable = true;
        width = 1;
        active.color = config.theme.palettes.dark.focus;
        inactive.color = config.theme.palettes.dark.bg;
      };
      # Turn off only the window-close animation
      animations = {
        window-close.enable = false;
        window-open.enable = false;
      };

      ### Keybindings
      binds =
        with config.lib.niri.actions;
        let
          mod = "Super";
        in
        {
          # Hotkey overlay
          "${mod}+Shift+Slash".action = show-hotkey-overlay;

          # Launchers
          "${mod}+Return".action = spawn "foot";
          "${mod}+D".action = spawn "vicinae" "toggle";
          "${mod}+Shift+D".action = spawn "yofi" "apps";
          "${mod}+P".action = spawn "wlr-which-key" "${config.theme.dir}/current/which-key-power.yaml";
          "${mod}+O".action = spawn "wlr-which-key" "${config.theme.dir}/current/which-key-opener.yaml";

          # Volume controls
          "XF86AudioRaiseVolume".allow-when-locked = true;
          "XF86AudioRaiseVolume".action = spawn "wpctl" "set-volume" "@DEFAULT_AUDIO_SINK@" "0.1+";
          "XF86AudioLowerVolume".allow-when-locked = true;
          "XF86AudioLowerVolume".action = spawn "wpctl" "set-volume" "@DEFAULT_AUDIO_SINK@" "0.1-";
          "XF86AudioMute".allow-when-locked = true;
          "XF86AudioMute".action = spawn "wpctl" "set-mute" "@DEFAULT_AUDIO_SINK@" "toggle";

          "XF86AudioMicMute".allow-when-locked = true;
          "XF86AudioMicMute".action = spawn "wpctl" "set-mute" "@DEFAULT_AUDIO_SOURCE@" "toggle";
          "XF86MonBrightnessUp".action = spawn "brightnessctl" "s" "10%+";
          "XF86MonBrightnessDown".action = spawn "brightnessctl" "s" "10%-";
          "Shift+XF86MonBrightnessUp".action = spawn "brightnessctl" "s" "1%+";
          "Shift+XF86MonBrightnessDown".action = spawn "brightnessctl" "s" "1%-";

          # Window management
          "${mod}+Shift+Q".action = close-window;

          # Focus
          "${mod}+H".action = focus-column-left;
          "${mod}+J".action = focus-window-down;
          "${mod}+K".action = focus-window-up;
          "${mod}+L".action = focus-column-right;
          "${mod}+Left".action = focus-column-left;
          "${mod}+Down".action = focus-window-down;
          "${mod}+Up".action = focus-window-up;
          "${mod}+Right".action = focus-column-right;

          # Move
          "${mod}+Ctrl+H".action = move-column-left;
          "${mod}+Ctrl+J".action = move-window-down-or-to-workspace-down;
          "${mod}+Ctrl+K".action = move-window-up-or-to-workspace-up;
          "${mod}+Ctrl+L".action = move-column-right;
          "${mod}+Ctrl+Left".action = move-column-left;
          "${mod}+Ctrl+Down".action = move-window-down-or-to-workspace-down;
          "${mod}+Ctrl+Up".action = move-window-up-or-to-workspace-up;
          "${mod}+Ctrl+Right".action = move-column-right;

          # Cycle columns/workspaces
          "${mod}+Home".action = focus-column-first;
          "${mod}+End".action = focus-column-last;
          "${mod}+Ctrl+Home".action = move-column-to-first;
          "${mod}+Ctrl+End".action = move-column-to-last;

          # Monitors
          "${mod}+Shift+H".action = focus-monitor-left;
          "${mod}+Shift+J".action = focus-monitor-down;
          "${mod}+Shift+K".action = focus-monitor-up;
          "${mod}+Shift+L".action = focus-monitor-right;
          "${mod}+Shift+Ctrl+H".action = move-column-to-monitor-left;
          "${mod}+Shift+Ctrl+J".action = move-column-to-monitor-down;
          "${mod}+Shift+Ctrl+K".action = move-column-to-monitor-up;
          "${mod}+Shift+Ctrl+L".action = move-column-to-monitor-right;

          # Workspaces
          "${mod}+U".action = focus-workspace-down;
          "${mod}+I".action = focus-workspace-up;
          # "${mod}+Ctrl+U".action = move-window-down-or-workspace-down;
          # "${mod}+Ctrl+I".action = move-window-up-or-workspace-up;
          "${mod}+Shift+U".action = move-workspace-down;
          "${mod}+Shift+I".action = move-workspace-up;

          # Scroll through workspaces
          "${mod}+WheelScrollDown" = {
            cooldown-ms = 150;
            action = focus-workspace-down;
          };
          "${mod}+WheelScrollUp" = {
            cooldown-ms = 150;
            action = focus-workspace-up;
          };
          "${mod}+Ctrl+WheelScrollDown" = {
            cooldown-ms = 150;
            action = move-column-to-workspace-down;
          };
          "${mod}+Ctrl+WheelScrollUp" = {
            cooldown-ms = 150;
            action = move-column-to-workspace-up;
          };

          # Scroll columns
          "${mod}+WheelScrollRight".action = focus-column-right;
          "${mod}+WheelScrollLeft".action = focus-column-left;
          "${mod}+Ctrl+WheelScrollRight".action = move-column-right;
          "${mod}+Ctrl+WheelScrollLeft".action = move-column-left;
          "${mod}+Shift+WheelScrollDown".action = focus-column-right;
          "${mod}+Shift+WheelScrollUp".action = focus-column-left;
          "${mod}+Ctrl+Shift+WheelScrollDown".action = move-column-right;
          "${mod}+Ctrl+Shift+WheelScrollUp".action = move-column-left;

          # Numeric workspaces
          "${mod}+1".action = focus-workspace 1;
          "${mod}+2".action = focus-workspace 2;
          "${mod}+3".action = focus-workspace 3;
          "${mod}+4".action = focus-workspace 4;
          "${mod}+5".action = focus-workspace 5;
          "${mod}+6".action = focus-workspace 6;
          "${mod}+7".action = focus-workspace 7;
          "${mod}+8".action = focus-workspace 8;
          "${mod}+9".action = focus-workspace 9;
          "${mod}+Ctrl+1".action = move-column-to-index 1;
          "${mod}+Ctrl+2".action = move-column-to-index 2;
          "${mod}+Ctrl+3".action = move-column-to-index 3;
          "${mod}+Ctrl+4".action = move-column-to-index 4;
          "${mod}+Ctrl+5".action = move-column-to-index 5;
          "${mod}+Ctrl+6".action = move-column-to-index 6;
          "${mod}+Ctrl+7".action = move-column-to-index 7;
          "${mod}+Ctrl+8".action = move-column-to-index 8;
          "${mod}+Ctrl+9".action = move-column-to-index 9;

          # Window grouping
          "${mod}+BracketLeft".action = consume-or-expel-window-left;
          "${mod}+BracketRight".action = consume-or-expel-window-right;
          "${mod}+Comma".action = consume-window-into-column;
          "${mod}+Period".action = expel-window-from-column;

          # Layout actions
          "${mod}+R".action = switch-preset-column-width;
          "${mod}+Shift+R".action = switch-preset-window-height;
          "${mod}+Ctrl+R".action = reset-window-height;
          "${mod}+F".action = maximize-column;
          "${mod}+Shift+F".action = fullscreen-window;
          "${mod}+Ctrl+F".action = expand-column-to-available-width;
          "${mod}+C".action = center-column;
          "${mod}+B".action = spawn "ironbar" "bar" "main" "toggle-visible";

          # Adjust dimensions
          "${mod}+Minus".action = set-column-width "-10%";
          "${mod}+Equal".action = set-column-width "+10%";
          "${mod}+Shift+Minus".action = set-window-height "-10%";
          "${mod}+Shift+Equal".action = set-window-height "+10%";

          # Floating/Tiling
          "${mod}+V".action = toggle-window-floating;
          "${mod}+Shift+V".action = switch-focus-between-floating-and-tiling;

          # Tabs
          "${mod}+W".action = toggle-column-tabbed-display;

          # Screenshots
          "Print".action.screenshot = [ ];
          # "Ctrl+Print".action = screenshot-screen;  # Doesnt work
          "Alt+Print".action.screenshot-window = [ ];

          # Inhibitor toggle
          "${mod}+Escape" = {
            allow-inhibiting = false;
            action = toggle-keyboard-shortcuts-inhibit;
          };

          # Quit & power
          "${mod}+Shift+E".action = quit;
          "Ctrl+Alt+Delete".action = quit;
          "${mod}+Shift+P".action = power-off-monitors;
        };

      ### Window‑specific rules
      "window-rules" = [
        {
          matches = [ { "app-id" = "foot"; } ];
          "default-column-width".proportion = 0.5;
        }
        {
          matches = [ { "app-id" = "chromium-browser"; } ];
          "default-column-width".proportion = 1.0;
        }
        {
          matches = [ { "app-id" = ".qemu-system-x86_64-wrapped"; } ];
          "default-column-width".fixed = 1280;
        }
        {
          matches = [ { "app-id" = "mpv"; } ];
          "default-column-width".proportion = 0.5;
        }
        {
          matches = [ { "app-id" = "rerun"; } ];
          "default-column-width".proportion = 1.0;
        }
        {
          matches = [
            { "app-id" = "^chrome-linear\\.app__"; }
            { "app-id" = "^dev\\.zed\\.Zed$"; }
          ];
          "default-column-width".proportion = 1.0;
        }
        {
          matches = [ { "app-id" = "(?i)^bitwarden$"; } ];
          "open-floating" = true;
          "open-maximized" = false;
          "open-focused" = true;
          "default-column-width".fixed = 1120;
          "default-window-height".fixed = 800;
        }
        # {
        #   matches = [ { "title" = "centerpiece"; } ];
        #   "open-floating" = true;
        # }
      ];
    };

    systemd.user.services.ironbar = {
      Unit = {
        Description = "Ironbar";
        PartOf = [ "graphical-session.target" ];
        After = [ "graphical-session.target" ];
        X-Restart-Triggers = [ config.xdg.configFile."ironbar/config.json".source ];
      };
      Service = {
        ExecStart = lib.getExe pkgs.ironbar;
        # Retries until ironbar's IPC socket is up.
        ExecStartPost = toString (
          pkgs.writeShellScript "ironbar-theme" ''
            for _ in {1..50}; do
              ${lib.getExe pkgs.ironbar} style load-css ${config.theme.dir}/ironbar.css && exit 0
              ${pkgs.coreutils}/bin/sleep 0.1
            done
          ''
        );
        Restart = "on-failure";
      };
      Install.WantedBy = [ "graphical-session.target" ];
    };

    xdg.configFile."ironbar/config.json".text =
      let
        # Dimmed by alpha rather than a colour, so it holds in both modes.
        label = text: "<span alpha='55%'>${text}</span>";
      in
      builtins.toJSON {
        name = "main";
        position = "bottom";
        layer = "bottom";
        height = 26;
        anchor_to_edges = true;

        # The machine on the left, ahead of the window title so it never
        # shifts; everything you act on on the right. A `cell` opens with a
        # divider.
        start = [
          {
            type = "workspaces";
            sort = "index";
          }
          {
            type = "label";
            class = "cell";
            label = "{{5000:${lib.getExe barSystem}}}";
          }
          {
            type = "focused";
            class = "cell";
            show_icon = false;
            truncate = {
              mode = "end";
              max_length = 80;
            };
          }
        ];
        end = [
          # Claudio plan usage. The usage endpoint is rate limited, so five
          # minutes is the lowest sane interval.
          {
            type = "script";
            class = "cell";
            cmd = lib.getExe barClaude;
            interval = 300000;
            tooltip = "{{300000:${lib.getExe barClaude} tooltip}}";
            on_click_left = "xdg-open https://claude.ai/settings/usage";
          }
          {
            type = "script";
            class = "cell";
            cmd = lib.getExe barNet;
            interval = 10000;
            on_click_left = "iwmenu -l custom --launcher-command \"${yofiIwmenu}\"";
          }
          {
            type = "volume";
            format = "${label "VOL"} {percentage}%";
            mute_format = "${label "VOL"} muted";
            show_sources = false;
          }
          {
            type = "battery";
            format = "${label "BAT"} {percentage}%";
            show_icon = false;
            use_default_profiles = false;
            profiles = {
              warning.when = {
                percent = 30;
                charging = false;
              };
              critical.when = {
                percent = 15;
                charging = false;
              };
            };
          }
          {
            type = "clock";
            class = "cell";
            format = "%a %d %b  %H:%M";
            format_popup = "%A %d %B %Y";
          }
        ];
      };

    # The dark sheet is the base; ExecStartPost loads the current mode's copy
    # over it, and ironbar reloads that copy whenever theme-apply rewrites it.
    xdg.configFile."ironbar/style.css".text = ironbarCss (
      config.theme.palettes.dark // { font = config.theme.fonts.${config.theme.defaultFont}; }
    );
    theme.files."ironbar.css" = ironbarCss;

    programs.imv = {
      enable = true;
      settings = {
        options = {
          overlay = "1";
          overlay_font = "Monospace:10";
        };
      };
    };

    programs.mpv = {
      enable = true;
      config = {
        hwdec = "auto-safe";
        vo = "gpu";
        profile = "gpu-hq";
        gpu-context = "wayland";
        sub-scale = 0.7;
        keep-open = true;
        # autoload the other files of the same type in the directory as a playlist
        autocreate-playlist = "same";
        osd-status-msg = "\${playback-time/full} / \${duration} (\${percent-pos}%)\\nframe: \${estimated-frame-number} / \${estimated-frame-count}";
        sub-file-paths = "Subs";
        save-position-on-quit = true;
        write-filename-in-watch-later-config = true;
      };
      scripts = [ pkgs.mpvScripts.memo ];
      bindings = {
        # increase subtitle font size
        "ALT+k" = "add sub-scale +0.1";
        # decrease subtitle font size
        "ALT+j" = "add sub-scale -0.1";
        # reduce video pan step size
        "ALT+LEFT" = "add video-pan-x +0.01";
        "ALT+RIGHT" = "add video-pan-x -0.01";
        "ALT+UP" = "add video-pan-y +0.01";
        "ALT+DOWN" = "add video-pan-y -0.01";
        # Disable scrolling, because it sucks
        MOUSE_BTN3 = "ignore";
        MOUSE_BTN4 = "ignore";
      };
    };
    programs.vicinae = {
      enable = true;
      systemd.enable = true;
      mutableSettings = true;
      settings = {
        tray.enabled = false;
        theme.dark.name = "terminal";
        theme.light.name = "terminal-light";
        imports = [ "${config.theme.dir}/current/vicinae.json" ];
        launcher_window = {
          opacity = 1;
          material = "none";
          rounding = 0;
          client_side_decorations = {
            border_width = 2;
            shadow_size = 0;
          };
          clock.format = "HH:mm";
        };
      };
      # The bar's look: flat, dim labels, One Dark / One Light accents.
      themes = {
        terminal = vicinaeTheme config.theme.palettes.dark;
        terminal-light = vicinaeTheme config.theme.palettes.light;
      };
    };
    theme.files."vicinae.json" = p: builtins.toJSON { font.normal.family = p.font; };
    # Quiet the file indexer. Release builds still `add_compile_definitions(DEBUG=0)`,
    # but the server gates on `#if defined(DEBUG)` — true either way — so it forces
    # VICINAE_FILE_INDEXER_LOG=debug onto the indexer unless the var is already set.
    systemd.user.services.vicinae.Service.Environment = [
      "VICINAE_FILE_INDEXER_LOG=warn"
    ];
  };
}
