import { NextResponse, type NextRequest } from "next/server";
import {
  AdminRequestError,
  apiErrorResponse,
  assertAdminMutation,
  readJsonObject,
} from "@/lib/admin-api";
import {
  addSiteSetting,
  deleteSiteSetting,
  renameSiteSetting,
  SiteSettingsValidationError,
  type SiteSettingKind,
  type SiteSettings,
} from "@/lib/site-settings";

export const dynamic = "force-dynamic";

/** Names are bounded at 80 characters; the extra KB covers the JSON envelope. */
const MAX_SETTINGS_REQUEST_BYTES = 2_048;
const NAME_KEYS: Record<string, true> = { kind: true, name: true };
const RENAME_KEYS: Record<string, true> = { kind: true, originalName: true, name: true };

/**
 * Runs one admin-only settings mutation. The store serializes writes and
 * returns the canonical lists, so every response replaces the client state
 * wholesale. Payload shape and value types are checked before the store is
 * touched, and no publication or settlement storage is rewritten.
 */
async function mutateSettings(
  request: NextRequest,
  allowedKeys: Record<string, true>,
  mismatchMessage: string,
  operation: (payload: Record<string, unknown>) => Promise<SiteSettings>,
) {
  try {
    await assertAdminMutation(request);
    const payload = await readJsonObject(request, MAX_SETTINGS_REQUEST_BYTES);
    const keys = Object.keys(payload);
    if (
      keys.length !== Object.keys(allowedKeys).length ||
      !keys.every((key) => allowedKeys[key] === true)
    ) {
      throw new AdminRequestError(mismatchMessage, 400);
    }

    const settings = await operation(payload);
    return NextResponse.json({ settings }, { headers: { "Cache-Control": "no-store" } });
  } catch (error) {
    if (error instanceof SiteSettingsValidationError) {
      return apiErrorResponse(new AdminRequestError(error.message, 400));
    }
    return apiErrorResponse(error);
  }
}

function readKind(value: unknown): SiteSettingKind {
  if (value !== "categories" && value !== "settlementTypes") {
    throw new AdminRequestError("Неизвестный список настроек.", 400);
  }
  return value;
}

/** Appends one name to a settings list; future forms offer it after a reload. */
export async function POST(request: NextRequest) {
  return mutateSettings(request, NAME_KEYS, "Ожидается название и список настроек.", (payload) =>
    addSiteSetting(readKind(payload.kind), payload.name),
  );
}

/** Renames one stored name in place; its position in the list is preserved. */
export async function PATCH(request: NextRequest) {
  return mutateSettings(
    request,
    RENAME_KEYS,
    "Ожидается исходное название, новое название и список настроек.",
    (payload) =>
      renameSiteSetting(readKind(payload.kind), payload.originalName, payload.name),
  );
}

/** Removes one stored name from future options; stored materials keep it. */
export async function DELETE(request: NextRequest) {
  return mutateSettings(request, NAME_KEYS, "Ожидается название и список настроек.", (payload) =>
    deleteSiteSetting(readKind(payload.kind), payload.name),
  );
}
