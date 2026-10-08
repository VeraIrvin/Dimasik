/** Default editorial categories; admins may append more in the site settings. */
export const POST_CATEGORIES = [
  "Статья",
  "Персона",
  "Ссылка на источник",
  "Источник с индексацией",
] as const;

/**
 * A category saved on a post. New posts pick from the site settings list,
 * while stored posts may carry any bounded string written under older settings.
 */
export type PostCategory = string;
