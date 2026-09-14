import { PaginationControls } from "@personal-cfo/desktop";

export function Default() {
  const pagination = {
    page: 0,
    pageCount: 13,
    pageSize: 10 as const,
    total: 128,
    rangeStart: 1,
    rangeEnd: 10,
    setPage: () => {},
    setPageSize: () => {},
  };
  return (
    <div style={{ width: 560 }}>
      <PaginationControls pagination={pagination} noun="transactions" />
    </div>
  );
}

export function MiddlePage() {
  const pagination = {
    page: 4,
    pageCount: 13,
    pageSize: 10 as const,
    total: 128,
    rangeStart: 41,
    rangeEnd: 50,
    setPage: () => {},
    setPageSize: () => {},
  };
  return (
    <div style={{ width: 560 }}>
      <PaginationControls pagination={pagination} noun="transactions" />
    </div>
  );
}

export function LastPage() {
  const pagination = {
    page: 12,
    pageCount: 13,
    pageSize: 10 as const,
    total: 128,
    rangeStart: 121,
    rangeEnd: 128,
    setPage: () => {},
    setPageSize: () => {},
  };
  return (
    <div style={{ width: 560 }}>
      <PaginationControls pagination={pagination} noun="transactions" />
    </div>
  );
}
