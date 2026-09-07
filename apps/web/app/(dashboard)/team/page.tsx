"use client";

import { useEffect, useState } from "react";
import {
  api,
  ApiError,
  ROUTING_MODES,
  type Member,
  type ProjectUsageResponse,
  type RoutingMode,
  type Team,
  type TeamMember,
} from "@/lib/api";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  ErrorState,
  Field,
  SectionHeader,
  Stat,
  TableShell,
  Td,
  Th,
} from "@/components/ui";
import { formatRelative, formatUsd } from "@/lib/format";

const ROUTING_MODE_LABEL: Record<RoutingMode, string> = {
  auto: "Auto",
  quality: "Quality",
  balanced: "Balanced",
  economy: "Economy",
  passthrough: "Passthrough",
};

const ROLES = [
  {
    id: "owner",
    label: "Owner",
    description: "Full control, including billing and deleting the organisation.",
  },
  {
    id: "admin",
    label: "Admin",
    description: "Manage keys, providers, policies and members. No billing changes.",
  },
  {
    id: "member",
    label: "Member",
    description: "Create and use API keys. Cannot change organisation settings.",
  },
  {
    id: "viewer",
    label: "Viewer",
    description: "Read-only. Sees usage and savings, changes nothing.",
  },
] as const;

/**
 * People and teams.
 *
 * Teams exist for two reasons that are easy to conflate: they scope budgets, and they are
 * the cost center on the chargeback report. Both are stated on the page, because an
 * organisation that names its teams after products gets a useful finance export for free
 * and one that names them after squads does not.
 */
export default function TeamPage() {
  const [members, setMembers] = useState<Member[]>([]);
  const [teams, setTeams] = useState<Team[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const [inviteEmail, setInviteEmail] = useState("");
  const [inviteRole, setInviteRole] = useState<string>("member");
  const [inviting, setInviting] = useState(false);
  const [lastInviteUrl, setLastInviteUrl] = useState<string | null>(null);
  const [copiedLink, setCopiedLink] = useState(false);

  const [teamName, setTeamName] = useState("");
  const [teamBudget, setTeamBudget] = useState("");
  const [teamMode, setTeamMode] = useState<RoutingMode | "">("");
  const [creatingTeam, setCreatingTeam] = useState(false);

  // The project (team) currently expanded below the table — its roster and usage load
  // lazily, on selection, rather than N+1 fetching every team up front for a page that
  // usually shows one org's handful of projects.
  const [managingTeam, setManagingTeam] = useState<Team | null>(null);
  const [teamMembers, setTeamMembers] = useState<TeamMember[]>([]);
  const [teamUsage, setTeamUsage] = useState<ProjectUsageResponse | null>(null);
  const [panelLoading, setPanelLoading] = useState(false);
  const [panelError, setPanelError] = useState<string | null>(null);
  const [newMemberId, setNewMemberId] = useState("");
  const [newMemberRole, setNewMemberRole] = useState<"lead" | "member">("member");
  const [addingMember, setAddingMember] = useState(false);

  async function load() {
    try {
      const [memberResponse, teamResponse] = await Promise.all([
        api.listMembers(),
        api.listTeams(),
      ]);
      setMembers(memberResponse.members);
      setTeams(teamResponse.teams);
      setError(null);
    } catch (caught) {
      setError(
        caught instanceof ApiError
          ? caught.message
          : "Could not load the organisation.",
      );
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  async function handleInvite(event: React.FormEvent) {
    event.preventDefault();
    if (!inviteEmail.trim()) return;

    setInviting(true);
    setNotice(null);
    setLastInviteUrl(null);
    try {
      const res = await api.inviteMember(inviteEmail.trim(), inviteRole);
      if (res && "invite_url" in res && res.invite_url) {
        try {
          const url = new URL(res.invite_url);
          url.protocol = window.location.protocol;
          url.host = window.location.host;
          setLastInviteUrl(url.toString());
        } catch {
          setLastInviteUrl(res.invite_url);
        }
      }
      if (res && "email_sent" in res && res.email_sent) {
        setNotice(`Invitation email sent to ${inviteEmail.trim()} via Resend!`);
      } else {
        setNotice(`Member added. Direct activation link generated for ${inviteEmail.trim()}.`);
      }
      setInviteEmail("");
      await load();
    } catch (caught) {
      setError(
        caught instanceof ApiError ? caught.message : "Could not send the invitation.",
      );
    } finally {
      setInviting(false);
    }
  }

  async function handleRemove(member: Member) {
    const confirmed = window.confirm(
      `Remove ${member.email}? Every API key they created is revoked at the same time — leaving those keys live is how a departed employee keeps access.`,
    );
    if (!confirmed) return;

    try {
      await api.removeMember(member.user_id);
      await load();
    } catch (caught) {
      setError(
        caught instanceof ApiError ? caught.message : "Could not remove the member.",
      );
    }
  }

  async function handleCreateTeam(event: React.FormEvent) {
    event.preventDefault();
    if (!teamName.trim()) return;

    const dollars = Number.parseFloat(teamBudget);
    const budget =
      teamBudget.trim() && Number.isFinite(dollars) && dollars > 0
        ? Math.round(dollars * 1_000_000)
        : null;

    setCreatingTeam(true);
    try {
      await api.createTeam(teamName.trim(), budget, teamMode || undefined);
      setTeamName("");
      setTeamBudget("");
      setTeamMode("");
      await load();
    } catch (caught) {
      setError(
        caught instanceof ApiError ? caught.message : "Could not create the team.",
      );
    } finally {
      setCreatingTeam(false);
    }
  }

  async function handleManage(team: Team) {
    if (managingTeam?.id === team.id) {
      setManagingTeam(null);
      return;
    }
    setManagingTeam(team);
    setPanelError(null);
    setPanelLoading(true);
    setNewMemberId("");
    try {
      const [membersRes, usageRes] = await Promise.all([
        api.listTeamMembers(team.id),
        api.projectUsage(team.id),
      ]);
      setTeamMembers(membersRes.members);
      setTeamUsage(usageRes);
    } catch (caught) {
      setPanelError(
        caught instanceof ApiError
          ? caught.message
          : "Could not load this project's roster and usage.",
      );
    } finally {
      setPanelLoading(false);
    }
  }

  async function handleAddTeamMember(event: React.FormEvent) {
    event.preventDefault();
    if (!managingTeam || !newMemberId) return;

    setAddingMember(true);
    setPanelError(null);
    try {
      await api.addTeamMember(managingTeam.id, newMemberId, newMemberRole);
      const membersRes = await api.listTeamMembers(managingTeam.id);
      setTeamMembers(membersRes.members);
      setNewMemberId("");
    } catch (caught) {
      setPanelError(
        caught instanceof ApiError ? caught.message : "Could not add that person.",
      );
    } finally {
      setAddingMember(false);
    }
  }

  async function handleRemoveTeamMember(member: TeamMember) {
    if (!managingTeam) return;
    const confirmed = window.confirm(
      `Remove ${member.email} from "${managingTeam.name}"? They keep their organisation access — this only removes them from the project.`,
    );
    if (!confirmed) return;

    try {
      await api.removeTeamMember(managingTeam.id, member.user_id);
      setTeamMembers((current) => current.filter((m) => m.user_id !== member.user_id));
    } catch (caught) {
      setPanelError(
        caught instanceof ApiError ? caught.message : "Could not remove that person.",
      );
    }
  }

  async function handleDeleteTeam(team: Team) {
    const confirmed = window.confirm(
      `Delete the team "${team.name}"? Keys assigned to it keep working but lose their cost-center attribution.`,
    );
    if (!confirmed) return;

    try {
      await api.deleteTeam(team.id);
      await load();
    } catch (caught) {
      setError(
        caught instanceof ApiError ? caught.message : "Could not delete the team.",
      );
    }
  }

  return (
    <>
      <SectionHeader
        eyebrow="Organisation"
        title="People and teams"
        description="Members sign in to this dashboard. Teams scope budgets and become the cost centers on the chargeback report, so naming them the way finance names things pays off later."
      />

      {error && (
        <div className="mb-6">
          <ErrorState message={error} />
        </div>
      )}

      {notice && (
        <div className="mb-6 rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface2)] px-4 py-3 text-xs font-bold text-[var(--color-ink)]">
          {notice}
        </div>
      )}

      {lastInviteUrl && (
        <div className="mb-6 rounded-2xl border border-[var(--color-accent)] bg-[var(--color-surface)] p-4 shadow-[2px_2px_0_var(--shadow-color)]">
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <div className="min-w-0 flex-1">
              <div className="text-xs font-bold text-[var(--color-accent)] uppercase tracking-wider mb-1">
                Direct Activation Link
              </div>
              <p className="font-mono text-xs text-[var(--color-ink)] truncate select-all bg-[var(--color-surface2)] px-3 py-1.5 rounded-lg border border-[var(--color-desk-line)]">
                {lastInviteUrl}
              </p>
            </div>
            <button
              type="button"
              onClick={() => {
                void navigator.clipboard.writeText(lastInviteUrl);
                setCopiedLink(true);
                setTimeout(() => setCopiedLink(false), 2000);
              }}
              className="shrink-0 rounded-xl bg-[var(--color-accent)] px-4 py-2 text-xs font-bold text-[var(--color-surface)] shadow-[2px_2px_0_var(--shadow-color)] hover:bg-[var(--color-accent-dark)]"
            >
              {copiedLink ? "Copied Link!" : "Copy Link"}
            </button>
          </div>
        </div>
      )}

      <Card className="mb-8 p-5">
        <h3 className="mb-4 text-sm font-bold text-[var(--color-ink)]">Invite a member</h3>
        <form
          onSubmit={handleInvite}
          className="grid gap-4 sm:grid-cols-[1fr_auto_auto] sm:items-end"
        >
          <Field
            label="Email address"
            id="invite-email"
            type="email"
            value={inviteEmail}
            onChange={setInviteEmail}
            required
            placeholder="colleague@company.com"
          />

          <div>
            <label
              htmlFor="invite-role"
              className="block text-sm font-medium text-[var(--color-muted)]"
            >
              Role
            </label>
            <select
              id="invite-role"
              value={inviteRole}
              onChange={(event) => setInviteRole(event.target.value)}
              className="mt-1.5 w-full rounded-[12px] border border-[var(--color-accent)] bg-[var(--color-surface2)] px-3 py-2 text-sm text-[var(--color-ink)]"
            >
              {ROLES.map((role) => (
                <option key={role.id} value={role.id}>
                  {role.label}
                </option>
              ))}
            </select>
          </div>

          <Button type="submit" disabled={inviting || !inviteEmail.trim()}>
            {inviting ? "Sending…" : "Send invite"}
          </Button>
        </form>

        <p className="mt-3 text-xs font-medium leading-relaxed text-[var(--color-muted-light)]">
          {ROLES.find((role) => role.id === inviteRole)?.description}
        </p>
      </Card>

      <div className="mb-10">
        <h3 className="mb-3 text-sm font-bold text-[var(--color-ink)]">Members</h3>
        {loading ? (
          <Card className="p-10 text-center text-xs font-bold text-[var(--color-muted-light)]">
            Loading members…
          </Card>
        ) : members.length === 0 ? (
          <EmptyState
            title="No members yet"
            description="You are the only person on this organisation. Invite a colleague above to give them dashboard access."
          />
        ) : (
          <TableShell>
            <thead>
              <tr>
                <Th>Member</Th>
                <Th>Role</Th>
                <Th>Joined</Th>
                <Th align="right">
                  <span className="sr-only">Actions</span>
                </Th>
              </tr>
            </thead>
            <tbody>
              {members.map((member) => (
                <tr key={member.user_id}>
                  <Td>
                    <div className="font-bold text-[var(--color-ink)]">
                      {member.name ?? member.email}
                    </div>
                    {member.name && (
                      <div className="text-[11px] text-[var(--color-muted-light)]">{member.email}</div>
                    )}
                  </Td>
                  <Td>
                    <Badge
                      tone={member.role === "owner" ? "accent" : "neutral"}
                      size="sm"
                    >
                      {member.role}
                    </Badge>
                  </Td>
                  <Td muted>{formatRelative(member.joined_at)}</Td>
                  <Td align="right">
                    {member.role !== "owner" && (
                      <Button variant="danger" onClick={() => handleRemove(member)}>
                        Remove
                      </Button>
                    )}
                  </Td>
                </tr>
              ))}
            </tbody>
          </TableShell>
        )}
      </div>

      <Card className="mb-6 p-5">
        <h3 className="mb-4 text-sm font-bold text-[var(--color-ink)]">Create a team</h3>
        <form
          onSubmit={handleCreateTeam}
          className="grid gap-4 sm:grid-cols-[1fr_1fr_1fr_auto] sm:items-end"
        >
          <Field
            label="Team name"
            id="team-name"
            value={teamName}
            onChange={setTeamName}
            required
            placeholder="e.g. Platform"
            hint="This becomes the cost center on chargeback reports."
          />
          <Field
            label="Monthly budget (USD)"
            id="team-budget"
            value={teamBudget}
            onChange={setTeamBudget}
            placeholder="Optional"
          />
          <div>
            <label
              htmlFor="team-mode"
              className="block text-sm font-medium text-[var(--color-muted)]"
            >
              Default routing mode
            </label>
            <select
              id="team-mode"
              value={teamMode}
              onChange={(event) => setTeamMode(event.target.value as RoutingMode | "")}
              className="mt-1.5 w-full rounded-[12px] border border-[var(--color-accent)] bg-[var(--color-surface2)] px-3 py-2 text-sm text-[var(--color-ink)]"
            >
              <option value="">Inherit from organisation</option>
              {ROUTING_MODES.map((mode) => (
                <option key={mode} value={mode}>
                  {ROUTING_MODE_LABEL[mode]}
                </option>
              ))}
            </select>
          </div>
          <Button type="submit" disabled={creatingTeam || !teamName.trim()}>
            {creatingTeam ? "Creating…" : "Create team"}
          </Button>
        </form>
        <p className="mt-3 text-xs font-medium leading-relaxed text-[var(--color-muted-light)]">
          The mode a key on this project falls back to when it sets no default of its own,
          and the caller sends no <code className="font-mono">X-Aegis-Routing-Hint</code>{" "}
          header.
        </p>
      </Card>

      {!loading &&
        (teams.length === 0 ? (
          <EmptyState
            title="No teams yet"
            description="Without teams, all spend lands in a single unattributed bucket. Create one per cost center to get a chargeback report finance can use directly."
          />
        ) : (
          <TableShell>
            <thead>
              <tr>
                <Th>Team</Th>
                <Th>Default mode</Th>
                <Th align="right">Monthly budget</Th>
                <Th>Created</Th>
                <Th align="right">
                  <span className="sr-only">Actions</span>
                </Th>
              </tr>
            </thead>
            <tbody>
              {teams.map((team) => (
                <tr key={team.id}>
                  <Td>{team.name}</Td>
                  <Td muted={!team.default_routing_mode}>
                    {team.default_routing_mode
                      ? ROUTING_MODE_LABEL[team.default_routing_mode]
                      : "inherited"}
                  </Td>
                  <Td align="right" mono muted={team.monthly_budget_mc === null}>
                    {team.monthly_budget_mc === null
                      ? "uncapped"
                      : formatUsd(team.monthly_budget_mc)}
                  </Td>
                  <Td muted>{formatRelative(team.created_at)}</Td>
                  <Td align="right">
                    <div className="flex justify-end gap-2">
                      <Button variant="secondary" onClick={() => handleManage(team)}>
                        {managingTeam?.id === team.id ? "Close" : "Manage"}
                      </Button>
                      <Button variant="danger" onClick={() => handleDeleteTeam(team)}>
                        Delete
                      </Button>
                    </div>
                  </Td>
                </tr>
              ))}
            </tbody>
          </TableShell>
        ))}

      {managingTeam && (
        <Card className="mt-6 p-5">
          <div className="mb-4 flex items-start justify-between gap-4">
            <div>
              <h3 className="text-sm font-bold text-[var(--color-ink)]">
                {managingTeam.name}
              </h3>
              <p className="mt-0.5 text-xs font-medium text-[var(--color-muted-light)]">
                Roster and this month&rsquo;s spend for this project.
              </p>
            </div>
            <Button variant="ghost" onClick={() => setManagingTeam(null)}>
              Close
            </Button>
          </div>

          {panelError && (
            <div className="mb-4">
              <ErrorState message={panelError} />
            </div>
          )}

          {panelLoading ? (
            <p className="py-6 text-center text-xs font-bold text-[var(--color-muted-light)]">
              Loading…
            </p>
          ) : (
            <>
              {teamUsage && (
                <div className="mb-6 grid gap-3 sm:grid-cols-3">
                  <Stat
                    label="This project spent"
                    value={formatUsd(teamUsage.summary.actual_cost_mc)}
                    sublabel={`${teamUsage.summary.requests.toLocaleString()} requests, last 30 days`}
                  />
                  <Stat
                    label="Saved"
                    value={formatUsd(teamUsage.summary.gross_savings_mc)}
                    sublabel={`${teamUsage.derived.savings_percent.toFixed(1)}% of what the requested models would have cost`}
                    accent
                  />
                  <Stat
                    label="Cache hit rate"
                    value={`${teamUsage.derived.cache_hit_rate.toFixed(1)}%`}
                  />
                </div>
              )}

              <h4 className="mb-2 text-xs font-bold uppercase tracking-wide text-[var(--color-muted)]">
                Members
              </h4>
              {teamMembers.length === 0 ? (
                <p className="mb-4 text-xs font-medium text-[var(--color-muted-light)]">
                  Nobody has been added to this project yet.
                </p>
              ) : (
                <ul className="mb-4 divide-y divide-[var(--color-line)] rounded-xl border border-[var(--color-line)]">
                  {teamMembers.map((member) => (
                    <li
                      key={member.user_id}
                      className="flex items-center justify-between gap-3 px-4 py-2.5"
                    >
                      <div className="min-w-0">
                        <div className="truncate text-xs font-bold text-[var(--color-ink)]">
                          {member.name ?? member.email}
                        </div>
                        {member.name && (
                          <div className="truncate text-[11px] text-[var(--color-muted-light)]">
                            {member.email}
                          </div>
                        )}
                      </div>
                      <div className="flex shrink-0 items-center gap-2">
                        <Badge tone={member.role === "lead" ? "accent" : "neutral"} size="sm">
                          {member.role}
                        </Badge>
                        <Button
                          variant="danger"
                          onClick={() => handleRemoveTeamMember(member)}
                        >
                          Remove
                        </Button>
                      </div>
                    </li>
                  ))}
                </ul>
              )}

              <form
                onSubmit={handleAddTeamMember}
                className="grid gap-3 sm:grid-cols-[1fr_auto_auto] sm:items-end"
              >
                <div>
                  <label
                    htmlFor="new-member"
                    className="block text-sm font-medium text-[var(--color-muted)]"
                  >
                    Add a person
                  </label>
                  <select
                    id="new-member"
                    value={newMemberId}
                    onChange={(event) => setNewMemberId(event.target.value)}
                    className="mt-1.5 w-full rounded-[12px] border border-[var(--color-accent)] bg-[var(--color-surface2)] px-3 py-2 text-sm text-[var(--color-ink)]"
                  >
                    <option value="">Choose an organisation member…</option>
                    {members
                      .filter(
                        (candidate) =>
                          !teamMembers.some((m) => m.user_id === candidate.user_id),
                      )
                      .map((candidate) => (
                        <option key={candidate.user_id} value={candidate.user_id}>
                          {candidate.name ?? candidate.email}
                        </option>
                      ))}
                  </select>
                </div>
                <div>
                  <label
                    htmlFor="new-member-role"
                    className="block text-sm font-medium text-[var(--color-muted)]"
                  >
                    Role
                  </label>
                  <select
                    id="new-member-role"
                    value={newMemberRole}
                    onChange={(event) =>
                      setNewMemberRole(event.target.value as "lead" | "member")
                    }
                    className="mt-1.5 w-full rounded-[12px] border border-[var(--color-accent)] bg-[var(--color-surface2)] px-3 py-2 text-sm text-[var(--color-ink)]"
                  >
                    <option value="member">Member</option>
                    <option value="lead">Lead</option>
                  </select>
                </div>
                <Button type="submit" disabled={addingMember || !newMemberId}>
                  {addingMember ? "Adding…" : "Add to project"}
                </Button>
              </form>
              <p className="mt-2 text-xs font-medium leading-relaxed text-[var(--color-muted-light)]">
                A lead may manage this project&rsquo;s keys, budget, and routing default
                without full organisation admin. A member has read access to the
                project&rsquo;s own usage only.
              </p>
            </>
          )}
        </Card>
      )}
    </>
  );
}
