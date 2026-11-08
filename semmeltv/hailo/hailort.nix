{
  lib,
  stdenv,
  fetchFromGitHub,
  fetchFromGitLab,
  cmake,
  ninja,
  generation,
}:

let
  fetchExternal =
    {
      gitlab ? false,
      ...
    }@spec:
    (if gitlab then fetchFromGitLab else fetchFromGitHub) (removeAttrs spec [ "gitlab" ]);
in
stdenv.mkDerivation {
  pname = "hailort";
  inherit (generation) version;

  src = fetchFromGitHub {
    owner = "hailo-ai";
    repo = "hailort";
    rev = "v${generation.version}";
    hash = generation.hailortHash;
  };

  postPatch = ''
    substituteInPlace hailort/cmake/execute_cmake.cmake \
      --replace-fail 'if(CMAKE_HOST_UNIX)' 'if(FALSE)'
    substituteInPlace hailort/cmake/external/protobuf.cmake \
      --replace-fail '-Dprotobuf_BUILD_TESTS:BOOL=OFF' \
        '-Dprotobuf_BUILD_TESTS:BOOL=OFF -DCMAKE_INSTALL_LIBDIR=lib'

    mkdir -p hailort/external
    ${lib.concatStrings (
      lib.mapAttrsToList (name: spec: ''
        cp -r --no-preserve=mode ${fetchExternal spec} hailort/external/${name}-src
      '') (removeAttrs generation.externals [ "pybind11" ])
    )}
  '';

  nativeBuildInputs = [
    cmake
    ninja
  ];

  cmakeFlags = [
    (lib.cmakeBool "HAILO_OFFLINE_COMPILATION" true)
    (lib.cmakeBool "HAILO_BUILD_EXAMPLES" false)
    (lib.cmakeFeature "CMAKE_POLICY_VERSION_MINIMUM" "3.5")
  ];

  postInstall = ''
    rm -r $out/lib/cmake/protobuf
  '';

  meta = {
    description = "Runtime library and CLI for Hailo AI accelerators";
    homepage = "https://github.com/hailo-ai/hailort";
    license = lib.licenses.mit;
    mainProgram = "hailortcli";
    platforms = lib.platforms.linux;
  };
}
