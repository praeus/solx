/**
 * React Query hooks wrapping the api.ts fetchers.
 *
 * Hook naming convention:
 *   useFoo()        — query hook
 *   useFooMutation  — mutation hook
 *
 * Each hook exposes React Query's full surface (data, isLoading, error, refetch…)
 * so callers don't need to import useQuery/useMutation directly.
 */

import { useMutation, useQuery, useQueryClient, type UseQueryResult } from "@tanstack/react-query";
import {
  ApiError,
  createDocument,
  deleteDocument,
  fetchConfig,
  fetchDocument,
  fetchDocuments,
  fetchHealth,
  fetchTypes,
  patchConfig,
  searchDocuments,
  type DocumentSummary,
  type HealthResponse,
  type Page,
  type SearchResults,
  type SolxConfig,
  type TypeEntity,
} from "./api";

/** ── Health ─────────────────────────────────────────────────────────── */

export function useHealth(): UseQueryResult<HealthResponse> {
  return useQuery({
    queryKey: ["health"],
    queryFn: fetchHealth,
    refetchInterval: 15_000,
    retry: false,
  });
}

/** ── Documents ──────────────────────────────────────────────────────── */

export interface UseDocumentsOptions {
  path_prefix?: string;
  limit?: number;
  offset?: number;
  sort_by?: string;
  sort_order?: "asc" | "desc";
  enabled?: boolean;
}

export function useDocuments(opts: UseDocumentsOptions = {}): UseQueryResult<Page<DocumentSummary>> {
  return useQuery({
    queryKey: ["documents", "list", opts],
    queryFn: () => fetchDocuments(opts),
    enabled: opts.enabled !== false,
    placeholderData: (previous) => previous,
  });
}

export interface UseSearchOptions {
  q?: string;
  path_prefix?: string;
  type_ref?: string;
  limit?: number;
  offset?: number;
  enabled?: boolean;
}

export function useSearchDocuments(opts: UseSearchOptions): UseQueryResult<SearchResults> {
  return useQuery({
    queryKey: ["documents", "search", opts],
    queryFn: () => searchDocuments(opts),
    enabled: opts.enabled !== false && Boolean(opts.q?.trim()),
    placeholderData: (previous) => previous,
  });
}

export function useDocument(path: string | undefined, name: string | undefined): UseQueryResult<DocumentSummary> {
  return useQuery({
    queryKey: ["documents", "item", path, name],
    queryFn: () => fetchDocument(path!, name!),
    enabled: Boolean(path) && Boolean(name),
  });
}

export function useCreateDocument() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: createDocument,
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["documents"] });
    },
  });
}

export function useDeleteDocument() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ path, name }: { path: string; name: string }) => deleteDocument(path, name),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["documents"] });
    },
  });
}

/** ── Types ──────────────────────────────────────────────────────────── */

export function useDocumentTypes(limit = 200): UseQueryResult<Page<TypeEntity>> {
  return useQuery({
    queryKey: ["types", "list", { limit }],
    queryFn: () => fetchTypes({ limit }),
    staleTime: 5 * 60 * 1000,
  });
}

/** ── Config ─────────────────────────────────────────────────────────── */

export function useConfig(): UseQueryResult<SolxConfig> {
  return useQuery({
    queryKey: ["config"],
    queryFn: fetchConfig,
    staleTime: 60_000,
  });
}

export function useMutateConfig() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (patch: Record<string, unknown>) => patchConfig(patch),
    onSuccess: (data) => {
      qc.setQueryData(["config"], data);
    },
  });
}

/** ── Re-exports ─────────────────────────────────────────────────────── */

export { ApiError };
