{
  lib,
  python3Packages,
  fetchFromGitHub,
}:

python3Packages.buildPythonApplication rec {
  pname = "workspace-mcp";
  version = "1.30.1";
  pyproject = true;

  src = fetchFromGitHub {
    owner = "taylorwilsdon";
    repo = "google_workspace_mcp";
    tag = "v${version}";
    hash = "sha256-VFPDLdno/N3UdznpQQZGtr0PpLGvhA8AqY3lVOQuXW8=";
  };

  build-system = with python3Packages; [
    setuptools
    wheel
  ];

  # Upstream's floors are security bumps a few patch releases ahead of nixpkgs.
  pythonRelaxDeps = [
    "pyjwt"
    "pypdf"
    "urllib3"
  ];

  dependencies = with python3Packages; [
    cryptography
    defusedxml
    email-validator
    fastapi
    fastmcp
    google-api-python-client
    google-auth-httplib2
    google-auth-oauthlib
    httpx
    markdown-it-py
    py-key-value-aio
    pyjwt
    pypdf
    python-dotenv
    pytz
    pyyaml
    tzdata
    urllib3
  ];

  pythonImportsCheck = [
    "main"
    "gdocs.docs_tools"
  ];

  meta = {
    description = "MCP server for Google Workspace: Docs, Sheets, Slides, Drive, Gmail, Calendar";
    homepage = "https://github.com/taylorwilsdon/google_workspace_mcp";
    license = lib.licenses.mit;
    mainProgram = "workspace-mcp";
  };
}
