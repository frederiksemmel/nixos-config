{
  curl,
  dejavu_fonts,
  makeFontsConf,
  mesa,
  playwright-driver,
  python3,
  runCommand,
  semmeltv-web,
}:

runCommand "semmeltv-web-browser-test"
  {
    nativeBuildInputs = [
      curl
      semmeltv-web
      (python3.withPackages (ps: [
        ps.aiohttp
        ps.playwright
      ]))
    ];
    PLAYWRIGHT_BROWSERS_PATH = playwright-driver.browsers;
    PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS = "true";
    FONTCONFIG_FILE = makeFontsConf { fontDirectories = [ dejavu_fonts ]; };
    __EGL_VENDOR_LIBRARY_DIRS = "${mesa}/share/glvnd/egl_vendor.d";
    LIBGL_ALWAYS_SOFTWARE = "1";
  }
  ''
    export HOME=$TMPDIR
    echo fake-service-token > $TMPDIR/service-token
    python3 -I ${./tests/browser}/fake_api.py 8090 &
    PORT=3000 SEMMELTV_SERVICE_TOKEN_FILE=$TMPDIR/service-token SEMMELTV_API_URL=http://127.0.0.1:8090 SEMMELTV_DOWNLOADS_URL=http://127.0.0.1:8090 semmeltv-web &
    curl --silent --retry 30 --retry-delay 1 --retry-connrefused --output /dev/null http://127.0.0.1:8090/api/auth/choices
    curl --silent --retry 30 --retry-delay 1 --retry-connrefused --output /dev/null http://127.0.0.1:3000/profiles
    python3 -I ${./tests/browser}/test.py http://127.0.0.1:3000 http://127.0.0.1:8090
    touch $out
  ''
