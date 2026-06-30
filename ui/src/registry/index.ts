/**
 * Built-in (first-party) widget registration.
 *
 * First-party widgets may be native Vue components or framework-agnostic
 * modules. Third-party widgets register their own namespaced types the same way
 * (or are loaded from a manifest `entry: { kind: "esm", source }`). See
 * CONTRIBUTING.md and docs/widget-authoring.md.
 */

import { registerVueWidget, registerModuleWidget } from "./registry";
import ChatWidget from "../widgets/ChatWidget.vue";
import EmotionWidget from "../widgets/EmotionWidget.vue";
import MemoryWidget from "../widgets/MemoryWidget.vue";
import InventoryWidget from "../widgets/InventoryWidget.vue";
import QuestWidget from "../widgets/QuestWidget.vue";
import MapWidget from "../widgets/MapWidget.vue";
import CardWidget from "../widgets/CardWidget.vue";
import { createClockWidget } from "../widgets/clock.module";

export function registerBuiltins(): void {
  registerVueWidget("core.chat", () => ChatWidget);
  registerVueWidget("core.emotion", () => EmotionWidget);
  registerVueWidget("core.memory", () => MemoryWidget);
  registerVueWidget("core.inventory", () => InventoryWidget);
  registerVueWidget("core.quest", () => QuestWidget);
  registerVueWidget("core.map", () => MapWidget);
  registerVueWidget("core.card", () => CardWidget);
  // A framework-agnostic (vanilla DOM) widget — proves authors aren't tied to Vue.
  registerModuleWidget("core.clock", () => createClockWidget());
}

export {
  registerWidget,
  registerVueWidget,
  registerModuleWidget,
  registerEsmWidget,
  setDefaultEsmImporter,
  resolveWidget,
  registeredTypes,
} from "./registry";
export {
  registerManifest,
  getManifest,
  allManifests,
  clearManifests,
  registerEsmWidgetsFromManifests,
  applyManifestMessage,
} from "./manifests";
export {
  needsConsent,
  canMount,
  effectiveCapabilities,
  grant,
  revoke,
  isGranted,
  clearGrants,
} from "./consent";
