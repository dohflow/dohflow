import { useCallback } from "react";

import { useNotices } from "@/components/ui/notice-context";

import {
  ANSWER_NOT_SAVED_MESSAGE,
  ANSWER_UNCONFIRMED_MESSAGE,
  linkFailedMessage,
  linkUnconfirmedMessage,
} from "./linkMessages";
import { useBills, type BillFollowUp } from "./useBills";

/// `useBills().addBill` plus the rule that a step after creation never fails silently
/// (docs/agent/FRONTEND.md section 6.2, personal-cfo-6wk.22 item 9): when the bill WAS created but
/// linking it, or saving the answer "it isn't that loan's payment", failed, an error notice says what
/// was done, what was not, and what to do. The bill stays; the result is returned unchanged.
export function useAddBill() {
  const { addBill } = useBills();
  const { notifyError } = useNotices();
  return useCallback(
    async (input: Parameters<typeof addBill>[0], follow?: BillFollowUp) => {
      const result = await addBill(input, follow);
      if (result.followUnconfirmed && follow) {
        notifyError(
          follow.linkLoanAccountId
            ? linkUnconfirmedMessage(follow.linkLoanName)
            : ANSWER_UNCONFIRMED_MESSAGE,
        );
      } else if (result.followError && follow) {
        notifyError(
          follow.linkLoanAccountId
            ? linkFailedMessage(follow.linkLoanName)
            : ANSWER_NOT_SAVED_MESSAGE,
        );
      }
      return result;
    },
    [addBill, notifyError],
  );
}
