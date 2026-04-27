# Mutual Non-Disclosure Agreement — Template

**This is a starting template, not signed legal advice.** Have a UK-qualified solicitor review before sending to any audit firm. Replace `[BRACKETED]` placeholders with real values.

---

## Mutual Non-Disclosure Agreement

This Mutual Non-Disclosure Agreement ("Agreement") is entered into on `[DATE]` between:

**Party A — "Discloser":**
- Name: Satyawan Singh, trading as EvaporChain (sole trader / `[OR LIMITED COMPANY DETAILS IF FORMED]`)
- Address: `[ADDRESS, LEICESTER, UK]`
- Email: `satyawansinghinuk@gmail.com`

**Party B — "Recipient":**
- Name: `[AUDIT FIRM LEGAL NAME]`
- Address: `[FIRM REGISTERED ADDRESS]`
- Authorised signatory: `[NAME, ROLE]`

Each a "Party" and together the "Parties".

---

### 1. Purpose

The Parties wish to discuss a potential security audit engagement covering the EvaporChain codebase ("Engagement"). To facilitate this discussion, each Party may share information that is confidential. This Agreement governs how that information is treated.

### 2. Definition of Confidential Information

"Confidential Information" means any non-public information disclosed by one Party to the other in any form (oral, written, electronic, source code, screen-shared, on physical media), whether marked "confidential" or not, including but not limited to:

- Source code, including any branch, fork, working copy, or fragment of the EvaporChain repository
- Architecture documents, threat models, audit reports (internal or external), test results, benchmarks
- Cryptographic keys, configuration files, deployment artefacts, infrastructure topology
- Business plans, financial projections, fundraising materials, customer/validator lists
- The fact that the Engagement is being discussed and the substance of those discussions

Confidential Information does not include information that:
- Is or becomes publicly available without breach of this Agreement
- Was already lawfully in the Recipient's possession prior to disclosure (with documentary evidence)
- Is independently developed by the Recipient without use of the Discloser's Confidential Information (with documentary evidence)
- Is required by law or court order to be disclosed (subject to §6 below)

### 3. Obligations

Each Party as Recipient agrees:

(a) To use the Discloser's Confidential Information solely for the Purpose stated in §1 and not for any other commercial, research, or personal purpose.

(b) To protect the Discloser's Confidential Information using the same degree of care it uses for its own confidential information of like importance, but in no event less than reasonable care.

(c) Not to disclose the Confidential Information to any third party without the Discloser's prior written consent, except to the Recipient's employees, contractors, and professional advisers who have a need to know for the Purpose and who are bound by confidentiality obligations at least as protective as this Agreement.

(d) Not to copy, reverse-engineer, decompile, or disassemble the Discloser's source code or other Confidential Information except as strictly necessary for the Purpose.

(e) To maintain a list of all individuals at the Recipient who have access to the Discloser's Confidential Information and to provide that list on request.

### 4. Source code handling — specific terms

Given that the EvaporChain source code is the central asset of this Engagement, the Parties additionally agree:

(a) Source code access will be granted by `[ACCESS METHOD — see ACCESS_PLAN.md, e.g., GitHub repo collaborator with read-only access; SCP push to firm-controlled host with named recipients; whatever is finalised]`.

(b) The Recipient shall not store the source code on personal devices, public cloud storage, or unencrypted media.

(c) On termination of the Engagement or this Agreement (whichever is earlier), the Recipient shall:
   - Delete all copies of the source code from its systems within 30 days
   - Provide written certification of deletion
   - Retain only the final delivered audit report and contemporaneous working notes, which remain subject to this Agreement

(d) The Recipient may retain a single archival copy of the final audit report for its own legal-record purposes for the period required by its professional indemnity policy or applicable law, whichever is longer.

### 5. Term

This Agreement comes into effect on the date written above and remains in effect for **three (3) years**, save that confidentiality obligations covering source code, cryptographic keys, and pre-mainnet deployment artefacts continue indefinitely.

### 6. Compelled disclosure

If a Recipient is required by law, regulator, or court order to disclose the Discloser's Confidential Information, the Recipient shall (where lawful) give the Discloser prompt written notice and reasonable cooperation in seeking a protective order or equivalent remedy.

### 7. No licence

Nothing in this Agreement grants either Party any licence to the other's intellectual property except as strictly necessary to perform the Purpose. The Discloser remains the owner of all source code, documentation, and pre-existing materials shared.

### 8. No representation as to accuracy

Each Discloser provides Confidential Information "as is". No representation or warranty is made as to its accuracy, completeness, or suitability for any purpose. Recipients act on the Confidential Information at their own risk for any purpose other than the agreed Purpose.

### 9. Remedies

The Parties acknowledge that breach of this Agreement may cause irreparable harm not adequately remedied by damages alone, and that the non-breaching Party shall be entitled to seek injunctive relief in addition to any other remedies available at law or in equity.

### 10. Governing law and jurisdiction

This Agreement is governed by the laws of **England and Wales**. The courts of England and Wales have exclusive jurisdiction to settle any dispute or claim arising out of or in connection with this Agreement.

### 11. Entire agreement

This Agreement constitutes the entire agreement between the Parties with respect to its subject matter and supersedes all prior discussions, arrangements, or understandings (oral or written).

### 12. Counterparts and signatures

This Agreement may be executed in counterparts (including electronic counterparts under the Electronic Communications Act 2000). PDF / DocuSign / equivalent signatures are acceptable.

---

**Discloser (Party A)**

Signed: ____________________________

Name: Satyawan Singh

Date: ______________________________


**Recipient (Party B)**

Signed: ____________________________

Name: `[FIRM REPRESENTATIVE]`

Title: `[ROLE]`

Date: ______________________________

---

## Notes for Satyawan before sending

1. **If still a sole trader,** sign in your own name with "trading as EvaporChain" — no company stamp needed.
2. **If you've formed a Limited company,** replace Party A details with the company's registered address and sign as a director. Recommended for any audit engagement above £50K.
3. **Insurance:** before signing, confirm the firm carries Professional Indemnity insurance ≥ engagement fee × 5. UK-based firms typically have £5-10M. Ask for the certificate.
4. **Variations:** firms often want to change §10 to their home jurisdiction. Hold the line on England and Wales unless the firm is willing to accept your jurisdiction in exchange for a fee discount.
5. **Tier 1 firms** (Trail of Bits, Sigma Prime, Zellic) have their own NDA template they prefer. It's faster to redline theirs than push yours, *unless* their template is materially looser on §4 (source code handling) or §5 (term). Compare side-by-side.
6. **Have a UK solicitor review** before first signature. The hourly rate for a one-shot review is typically £200-400 and saves disputes later.
