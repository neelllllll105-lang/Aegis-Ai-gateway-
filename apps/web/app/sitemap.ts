import type { MetadataRoute } from "next";

/**
 * Sitemap for the marketing site.
 *
 * Only public pages appear. Authenticated routes are excluded here, disallowed in
 * `public/robots.txt`, and marked `noindex` by the dashboard layout — three layers,
 * because a sitemap listing a page a crawler cannot reach is a crawl error, and a
 * dashboard URL appearing in a search result is worse than that.
 *
 * `changeFrequency` and `priority` are advisory and largely ignored by Google, so they
 * are set to something honest rather than to whatever would be flattering.
 */
export default function sitemap(): MetadataRoute.Sitemap {
  const base = process.env.NEXT_PUBLIC_SITE_URL ?? "https://aegis.dev";
  const lastModified = new Date();

  return [
    { url: base, lastModified, changeFrequency: "weekly", priority: 1 },
    {
      url: `${base}/pricing`,
      lastModified,
      changeFrequency: "weekly",
      priority: 0.9,
    },
    {
      url: `${base}/docs`,
      lastModified,
      changeFrequency: "weekly",
      priority: 0.8,
    },
    {
      url: `${base}/connect`,
      lastModified,
      changeFrequency: "monthly",
      priority: 0.8,
    },
    {
      url: `${base}/faq`,
      lastModified,
      changeFrequency: "monthly",
      priority: 0.7,
    },
  ];
}
