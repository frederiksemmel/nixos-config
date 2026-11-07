{
  lib,
  buildPythonPackage,
  fetchFromGitHub,
  python,
  cmake,
  hailort,
  generation,
}:

buildPythonPackage {
  pname = "pyhailort";
  inherit (hailort) version src;
  pyproject = false;

  postPatch = ''
    mkdir -p hailort/external
    cp -r --no-preserve=mode ${fetchFromGitHub generation.externals.pybind11} hailort/external/pybind11-src
  '';

  nativeBuildInputs = [ cmake ];
  buildInputs = [ hailort ];
  dependencies = map (name: python.pkgs.${name}) generation.pythonDependencies;

  cmakeDir = "../hailort/libhailort/bindings/python/src";
  cmakeFlags = [
    (lib.cmakeBool "FETCHCONTENT_FULLY_DISCONNECTED" true)
    (lib.cmakeFeature "HAILORT_VERSION" hailort.version)
    (lib.cmakeFeature "PYBIND11_PYTHON_VERSION" python.pythonVersion)
    (lib.cmakeFeature "PYTHON_EXECUTABLE" python.interpreter)
  ];

  installPhase = ''
    runHook preInstall
    site=$out/${python.sitePackages}
    mkdir -p $site
    cp -r ../hailort/libhailort/bindings/python/platform/hailo_platform $site/
    install -m555 _pyhailort*.so $site/hailo_platform/pyhailort/
    mkdir $site/hailort-${hailort.version}.dist-info
    cat > $site/hailort-${hailort.version}.dist-info/METADATA <<EOF
    Metadata-Version: 2.1
    Name: hailort
    Version: ${hailort.version}
    EOF
    runHook postInstall
  '';

  pythonImportsCheck = [ "hailo_platform" ];

  meta = {
    description = "Python bindings for HailoRT";
    homepage = "https://github.com/hailo-ai/hailort";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
  };
}
