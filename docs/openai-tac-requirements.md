# OpenAI TAC Access Requirements

Tracking note, recorded 2026-08-10. We intend to use OpenAI's cyber security
platform (TAC) to evaluate our product. Access requires attesting to the
customer-side requirements below. **Before executing anything inside our
environment with their tools, verify we conform to every item on this list.**

The items are transcribed from OpenAI's attestation form (checkbox text,
long form):

1. **Security certification** — Customer maintains an enterprise security
   program with SOC 2 Type II, ISO 27001 or an equivalent certification.
2. **Access controls** — Customer maintains controls for single sign-on (SSO),
   multi-factor authentication (MFA), least-privileged role management and
   role-based access control (RBAC) for all workspaces with TAC access.
3. **API key governance** — Customer enforces API key governance, including
   vaulted keys, rotation and revocation procedures, per-service principals,
   least-privilege access, and scoped permissions.
4. **Abuse monitoring and log retention** — Customer monitors employee use of
   the Services for cyber abuse and retains logs of model usage sufficient to
   support retrospective review of violative prompts and outputs, where
   feasible and lawful.
5. **Incident response** — Customer maintains a documented incident response
   process for responding to incidents of account compromise or misuse.
6. **Separate TAC organization** — When requested by OpenAI, Customer will
   create and maintain a separate organization ID designated solely for use by
   Customer employees who are authorized to access TAC and will provide that
   organization ID to OpenAI.
7. **Domain-specific emails** — Customer's employees with access to TAC will
   use domain-specific emails to access the Services.
8. **Enterprise-managed devices** — Customer provides TAC access only through
   enterprise-controlled devices with standard security controls including
   disk encryption, patch management, endpoint protection and endpoint
   management.

## Status

Not yet reviewed against our environment. Each item needs a conformance check
(and remediation where needed) before first use of TAC tooling here.
