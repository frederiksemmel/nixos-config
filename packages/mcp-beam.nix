{
  lib,
  buildGoModule,
  fetchFromGitHub,
  makeWrapper,
  ffmpeg,
}:

buildGoModule {
  pname = "mcp-beam";
  version = "0-unstable-2026-10-10";

  src = fetchFromGitHub {
    owner = "alexballas";
    repo = "mcp-beam";
    rev = "32e02b012fdb7c489249e0832a0f8db7f1ab0f23";
    hash = "sha256-Wfut138tyHMk311BvGOiGc6PBvVzcsiO0c3+d8Lxpjg=";
  };

  vendorHash = "sha256-QYOaWrrVIw10w8RV3qqQ5TI4Fs6hm8Hj2jQWlphqH7M=";

  subPackages = [ "." ];

  nativeBuildInputs = [ makeWrapper ];

  postFixup = ''
    wrapProgram $out/bin/mcp-beam --suffix PATH : ${lib.makeBinPath [ ffmpeg ]}
  '';

  meta = {
    description = "MCP server for casting media to Chromecast and DLNA/UPnP devices";
    homepage = "https://github.com/alexballas/mcp-beam";
    license = lib.licenses.mit;
    mainProgram = "mcp-beam";
  };
}
