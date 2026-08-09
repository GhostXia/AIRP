import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const workflow = fs.readFileSync(
  path.join(ROOT, ".github", "workflows", "webui-windows-build.yml"),
  "utf8",
);
const readme = fs.readFileSync(path.join(ROOT, "tools", "dep-governance", "README.md"), "utf8");
const security = fs.readFileSync(path.join(ROOT, "docs", "SECURITY.md"), "utf8");
const risk = fs.readFileSync(path.join(ROOT, "docs", "RISK-REGISTER.md"), "utf8");
const devGuide = fs.readFileSync(path.join(ROOT, "docs", "DEV-GUIDE.md"), "utf8");
const baseline = fs.readFileSync(path.join(ROOT, "docs", "CURRENT-BASELINE.md"), "utf8");
const productionPlan = fs.readFileSync(path.join(ROOT, "docs", "WEBUI-PRODUCTION-PLAN.md"), "utf8");

test("workflow exposes explicit manual release inputs and no published pre-gate", () => {
  assert.match(workflow, /workflow_dispatch:\s+inputs:/);
  assert.match(workflow, /publish_release:\s+[\s\S]*?type:\s+boolean/);
  assert.match(workflow, /release_tag:\s+[\s\S]*?type:\s+string/);
  assert.doesNotMatch(workflow, /release:\s*\n\s+types:\s*\[published\]/);
  assert.doesNotMatch(workflow, /github\.event\.release/);
});

test("release validation can inspect drafts without widening ordinary packaging", () => {
  const releaseContextStart = workflow.indexOf("  release-context:");
  const validatePackageStart = workflow.indexOf("  validate-package:");
  const publishReleaseStart = workflow.indexOf("  publish-release:");
  assert.ok(releaseContextStart >= 0 && validatePackageStart > releaseContextStart && publishReleaseStart > validatePackageStart);
  const releaseContext = workflow.slice(releaseContextStart, validatePackageStart);
  const validatePackage = workflow.slice(validatePackageStart, publishReleaseStart);
  const publishRelease = workflow.slice(publishReleaseStart);

  assert.match(releaseContext, /if:\s+github\.event_name == 'workflow_dispatch' && inputs\.publish_release == true/);
  assert.match(releaseContext, /permissions:\s*\r?\n\s+contents: write/);
  assert.doesNotMatch(validatePackage, /contents:\s+write/);
  assert.doesNotMatch(validatePackage, /permissions:/);
  assert.match(validatePackage, /needs:\s+release-context/);
  assert.match(validatePackage, /always\(\)[\s\S]*needs\.release-context\.result == 'success'/);
  assert.match(publishRelease, /environment:\s+release/);
  assert.match(publishRelease, /permissions:\s*[\s\S]*contents: write/);
});

test("workflow validates exact tag and existing draft before package", () => {
  assert.match(workflow, /inputs\.release_tag/);
  assert.match(workflow, /fetch-depth:\s+0/);
  assert.match(workflow, /git check-ref-format [^\n]*refs\/tags/);
  assert.match(workflow, /git rev-parse --verify [^\n]*HEAD/);
  assert.match(workflow, /refs\/tags\/\$releaseTag\^\{commit\}/);
  assert.match(workflow, /gh release view \$releaseTag --json isDraft,tagName/);
  assert.match(workflow, /not a draft/);
});

test("publish job waits for validation and uses hosted approval", () => {
  assert.match(workflow, /publish-release:[\s\S]*?needs:\s+validate-package/);
  assert.match(workflow, /publish-release:[\s\S]*?environment:\s+release/);
  assert.match(workflow, /docs\/sbom\/audit-signoffs\.json/);
  assert.match(workflow, /validate-release-signoffs\.mjs/);
  assert.match(workflow, /--fail-on-block/);
  assert.match(workflow, /--fail-on-unknown/);
});

test("publish job rechecks draft/tag, rejects duplicates, and never clobbers", () => {
  assert.match(workflow, /Recheck tag commit and draft release/);
  assert.match(workflow, /Reject duplicate release assets/);
  assert.match(workflow, /gh release upload/);
  assert.doesNotMatch(workflow, /--clobber/);
  assert.match(workflow, /gh release edit \"\$RELEASE_TAG\" --draft=false/);
  assert.match(workflow, /release\/sbom\/inventory\.json/);
  assert.match(workflow, /release\/sbom\/airp\.spdx\.json/);
  assert.match(workflow, /release\/sbom\/airp\.cdx\.json/);
  assert.match(workflow, /release\/sbom\/THIRD-PARTY-NOTICES\.txt/);
  assert.doesNotMatch(workflow, /release\/sbom\/inventory\.txt/);
});

test("docs describe workflow and retain hosted-environment residual", () => {
  assert.match(readme, /publish_release=true/);
  assert.match(readme, /17 audit-required/);
  assert.match(readme, /four allowlisted SBOM\s+assets/);
  assert.match(security, /There is no `release: published` pre-publish gate/);
  assert.match(security, /Hosted environment configuration and a successful publish proof are not present/);
  assert.match(risk, /do not mark RR-008 fully mitigated until that evidence exists/);
  assert.match(risk, /environment's required-reviewer policy and hosted evidence are not verified/);
  assert.match(devGuide, /#527.*workflow.*exact-tag validation\/publish code gate/);
  assert.match(baseline, /#413\/\#527.*workflow.*exact-tag validation\/publish code gate/);
  assert.match(productionPlan, /#527.*exact-tag SBOM.*手动发布路径/);
  assert.match(productionPlan, /hosted environment approval 配置/);
});
