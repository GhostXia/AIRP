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
const projectReadme = fs.readFileSync(path.join(ROOT, "README.md"), "utf8");
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

test("release validation keeps package/smoke gates without audit generation", () => {
  assert.match(workflow, /publish-release:[\s\S]*?needs:\s+validate-package/);
  assert.match(workflow, /publish-release:[\s\S]*?environment:\s+release/);
  assert.match(workflow, /Smoke packaged engine and real Chrome[\s\S]*?-BrowserSmoke/);
  assert.match(workflow, /Smoke desktop UI inside the package/);
  assert.doesNotMatch(workflow, /discover-deps\.mjs|generate-sbom\.mjs|validate-release-signoffs\.mjs/);
  assert.doesNotMatch(workflow, /airp-release-sbom|release\/sbom|docs\/sbom\/audit-signoffs\.json/);
  assert.doesNotMatch(workflow, /--fail-on-block|--fail-on-unknown/);
});

test("publish job rechecks draft/tag, rejects duplicates, and never clobbers", () => {
  assert.match(workflow, /Recheck tag commit and draft release/);
  assert.match(workflow, /Reject pre-existing release assets/);
  assert.match(workflow, /gh release upload/);
  assert.doesNotMatch(workflow, /--clobber/);
  assert.match(workflow, /gh release edit \"\$RELEASE_TAG\" --draft=false/);
  assert.match(workflow, /gh release upload \"\$RELEASE_TAG\"[\s\\]+release\/package\/airp-webui-windows-x64\.zip/);
  assert.doesNotMatch(workflow, /inventory\.json|airp\.spdx\.json|airp\.cdx\.json|THIRD-PARTY-NOTICES\.txt/);
  const duplicateGuardStart = workflow.indexOf("      - name: Reject pre-existing release assets");
  const uploadStart = workflow.indexOf("      - name: Upload release assets without overwrite");
  assert.ok(duplicateGuardStart >= 0 && uploadStart > duplicateGuardStart);
  const duplicateGuard = workflow.slice(duplicateGuardStart, uploadStart);
  assert.match(duplicateGuard, /existing_asset_count=\"\$\(jq '\.assets \| length'/);
  assert.match(duplicateGuard, /if \[ \"\$existing_asset_count\" -ne 0 \]/);
  assert.match(duplicateGuard, /must not contain pre-existing assets/);
  assert.doesNotMatch(duplicateGuard, /for asset in/);
});

test("docs describe workflow and retain hosted-environment residual", () => {
  assert.match(readme, /publish_release=true/);
  assert.match(readme, /do not generate or upload/);
  assert.match(readme, /not release\s+attachments or CI approval gates/);
  assert.match(projectReadme, /只上传 Windows 便携包/);
  assert.match(security, /There is no `release: published` pre-publish gate/);
  assert.match(security, /Hosted environment configuration and a successful publish proof are not present/);
  assert.match(risk, /do not mark RR-008 fully mitigated until that evidence exists/);
  assert.match(risk, /Keep package\/browser\/desktop smoke and dependency snapshot review explicit/);
  assert.match(devGuide, /#527.*workflow.*exact-tag validation\/publish code gate/);
  assert.match(baseline, /#413\/\#527.*workflow.*exact-tag validation\/publish code gate/);
  assert.match(productionPlan, /#527.*exact-tag package validation/);
  assert.match(productionPlan, /hosted environment approval 配置/);
  const activeDocs = [readme, projectReadme, security, risk, devGuide, baseline, productionPlan].join("\n");
  assert.doesNotMatch(activeDocs, /four allowlisted SBOM\s+assets|5 assets|5 个资产/);
});
