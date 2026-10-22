include "required";

def results: "success", "failure", "cancelled", "skipped", null;
def selections: "true", "false", "", "invalid", true, false, null;
def fixture($selection; $checks; $job; $result):
  {checks: {result: $checks, outputs: {rust: $selection}}, ($job): {result: $result}};
def valid($selection; $checks; $result):
  $checks == "success" and
  (($selection == "true" and $result == "success") or
   ($selection == "false" and $result == "skipped"));
def check($claim; $message): if $claim then true else error($message) end;

[
([selections as $selection | results as $checks | results as $result |
  fixture($selection; $checks; "rust"; $result) |
  check(required == valid($selection; $checks; $result); "routine result mismatch")] | all),
([selections as $selection | results as $checks | results as $result |
  fixture($selection; $checks; "analyze"; $result) |
  check(codeql == valid($selection; $checks; $result); "CodeQL result mismatch")] | all),
([selections as $selection | results as $checks | results as $result | results as $review |
  fixture($selection; $checks; "cargo-deny"; $result) + {"dependency-review": {result: $review}} |
  check(security == (valid($selection; $checks; $result) and $review == "success"); "security result mismatch")] | all),
(["workflow-quality", "licensing", "windows-quality", "windows-latest", "msrv",
  "target-checks", "portable", "miri", "coverage", "public-api", "semver", "package-consumer"] as $jobs |
 [range(0; 4096) as $mask | ("failure", "cancelled", "skipped", null) as $bad |
  reduce range(0; $jobs | length) as $index ({};
    . + {($jobs[$index]): {result: (if ($mask / pow(2; $index) | floor) % 2 == 0 then "success" else $bad end)}}) |
  check(assurance == ($mask == 0); "candidate result mismatch")] | all),
([{}, {checks: {}}, {checks: {result: "success"}},
  {checks: {result: "success", outputs: {rust: "false"}}}] |
 all(.[]; (required | not) and (security | not) and (codeql | not) and (assurance | not))),
([fixture("true"; "failure"; "rust"; "success"),
  fixture("true"; "success"; "rust"; "skipped"),
  fixture("invalid"; "success"; "rust"; "success")] |
 check(all(.[]; required | not); "rejecting controls accepted"))
] | check(all; "missing metadata accepted")
