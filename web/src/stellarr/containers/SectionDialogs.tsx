import { useMemo } from "react";

import { sectionChannels } from "@/components/settings/channel/channelSettingsModel";
import { SectionCreate } from "@/components/settings/channel/SectionCreate";
import {
  SectionSettings,
  type SectionSettingsVM,
} from "@/components/settings/channel/SectionSettings";
import { ApiError, describeError } from "@/lib/api/errors";
import type { Role } from "@/ports";
import { useDirectory, useLanguage, usePorts, useUi } from "@/state/hooks";

import { usePermissionOverrides } from "./usePermissionOverrides";

/** The cog on a rail section header: its name, permissions and deletion. */
export function SectionSettingsDialog({ roles }: { roles: readonly Role[] }) {
  const { admin } = usePorts();
  const directory = useDirectory();
  const ui = useUi();

  const sectionId = ui.modal === "section" ? ui.sectionSettingsId : null;
  const { overrides, save } = usePermissionOverrides("section", sectionId);

  // The channels come along so the dialog knows, before anyone presses
  // delete, that a section holding channels cannot be deleted.
  const section = useMemo<SectionSettingsVM | null>(() => {
    const found = directory.sections.find((item) => item.id === ui.sectionSettingsId);
    if (found === undefined) return null;
    return {
      id: found.id,
      name: found.name,
      channels: sectionChannels(directory.channels, found.id),
    };
  }, [directory.sections, directory.channels, ui.sectionSettingsId]);

  return (
    <SectionSettings
      open={ui.modal === "section"}
      onClose={ui.closeModal}
      section={section}
      supported={admin.supported.sections}
      permissionsSupported={admin.supported.roles}
      roles={roles}
      permissionOverrides={overrides}
      onSave={async (patch) => {
        if (sectionId !== null) await admin.updateSection(sectionId, patch);
      }}
      onSavePermissionOverride={save}
      onDelete={async () => {
        if (sectionId === null) return "deleted";
        try {
          await admin.deleteSection(sectionId);
        } catch (cause) {
          // The server refuses while channels still sit under the section,
          // which the directory had not heard of yet.
          if (cause instanceof ApiError && cause.status === 409) return "blocked";
          throw cause;
        }
        ui.closeModal();
        return "deleted";
      }}
      onOpenRoles={() => ui.openModal("settings")}
    />
  );
}

/** "Add section" in the rail. */
export function SectionCreateDialog() {
  const { admin } = usePorts();
  const directory = useDirectory();
  const ui = useUi();
  const { t } = useLanguage();

  return (
    <SectionCreate
      open={ui.modal === "section-new"}
      onClose={ui.closeModal}
      serverName={directory.server.name}
      sections={directory.sections}
      channels={directory.channels}
      onCreate={async (input) => {
        try {
          await admin.createSection(input);
        } catch (cause) {
          throw new Error(describeError(cause, t("section.createFailed"), t));
        }
      }}
    />
  );
}
