/** District addresses follow the same slug grammar as other public entity pages. */
export const DISTRICT_URL_PREFIX = "/uezd/";
export const DISTRICT_URL_SLUG_MAX_LENGTH = 100;

const DISTRICT_SLUG_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

export function isValidDistrictSlug(slug: string): boolean {
  return (
    slug.length > 0 &&
    slug.length <= DISTRICT_URL_SLUG_MAX_LENGTH &&
    DISTRICT_SLUG_PATTERN.test(slug)
  );
}

export function districtUrlForSlug(slug: string): string {
  return `${DISTRICT_URL_PREFIX}${slug}`;
}
