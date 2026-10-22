def selected($job):
  .checks.result == "success" and
  (.checks.outputs.rust == "true" or .checks.outputs.rust == "false") and
  .[$job].result == (if .checks.outputs.rust == "true" then "success" else "skipped" end);

def required: selected("rust");
def security: selected("cargo-deny") and .["dependency-review"].result == "success";
def codeql: selected("analyze");

def assurance:
  . as $needs |
  ["workflow-quality", "licensing", "windows-quality", "windows-latest", "msrv",
   "target-checks", "portable", "miri", "coverage", "public-api", "semver", "package-consumer"] |
  all(.[]; $needs[.].result == "success");
