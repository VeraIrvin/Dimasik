/**
 * Settlement page addresses are stored as the fixed `/naselennyy-punkt/` prefix
 * plus a slug. This mirrors the Rust store's `is_valid_slug` /
 * `validate_settlement_url`, so the creation form validates and assembles the
 * same address the API accepts.
 */
export const SETTLEMENT_URL_PREFIX = "/naselennyy-punkt/";
export const SETTLEMENT_URL_SLUG_MAX_LENGTH = 100;

/** Lower-case Latin letters and digits, hyphen-separated into non-empty parts. */
const SETTLEMENT_SLUG_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

export function isValidSettlementSlug(slug: string): boolean {
  return (
    slug.length > 0 &&
    slug.length <= SETTLEMENT_URL_SLUG_MAX_LENGTH &&
    SETTLEMENT_SLUG_PATTERN.test(slug)
  );
}

export function settlementUrlForSlug(slug: string): string {
  return `${SETTLEMENT_URL_PREFIX}${slug}`;
}
