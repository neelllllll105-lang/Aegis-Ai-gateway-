"use client";

import { useEffect, useState } from "react";
import { api, ApiError, type Member, type Team } from "@/lib/api";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  ErrorState,
  Field,
  SectionHeader,
  TableShell,
  Td,
  Th,
} from "@/components/ui";
import { formatRelative, formatUsd } from "@/lib/format";

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

  const [teamName, setTeamName] = useState("");
  const [teamBudget, setTeamBudget] = useState("");
  const [creatingTeam, setCreatingTeam] = useState(false);

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
    try {
      await api.inviteMember(inviteEmail.trim(), inviteRole);
      setNotice(`Invitation sent to ${inviteEmail.trim()}.`);
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
      await api.createTeam(teamName.trim(), budget);
      setTeamName("");
      setTeamBudget("");
      await load();
    } catch (caught) {
      setError(
        caught instanceof ApiError ? caught.message : "Could not create the team.",
      );
    } finally {
      setCreatingTeam(false);
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
        <div className="mb-6 rounded-2xl border border-[#D9CFC7] bg-[#EFE9E3] px-4 py-3 text-xs font-bold text-black">
          {notice}
        </div>
      )}

      <Card className="mb-8 p-5">
        <h3 className="mb-4 text-sm font-black text-black">Invite a member</h3>
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
              className="block text-sm font-medium text-[#403B35]"
            >
              Role
            </label>
            <select
              id="invite-role"
              value={inviteRole}
              onChange={(event) => setInviteRole(event.target.value)}
              className="mt-1.5 w-full rounded-[12px] border border-[#C9B59C] bg-[#F9F8F6] px-3 py-2 text-sm text-[#0A0A0A]"
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

        <p className="mt-3 text-xs font-medium leading-relaxed text-[#70685E]">
          {ROLES.find((role) => role.id === inviteRole)?.description}
        </p>
      </Card>

      <div className="mb-10">
        <h3 className="mb-3 text-sm font-black text-black">Members</h3>
        {loading ? (
          <Card className="p-10 text-center text-xs font-bold text-[#70685E]">
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
                    <div className="font-bold text-black">
                      {member.name ?? member.email}
                    </div>
                    {member.name && (
                      <div className="text-[11px] text-[#70685E]">{member.email}</div>
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
        <h3 className="mb-4 text-sm font-black text-black">Create a team</h3>
        <form
          onSubmit={handleCreateTeam}
          className="grid gap-4 sm:grid-cols-[1fr_1fr_auto] sm:items-end"
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
          <Button type="submit" disabled={creatingTeam || !teamName.trim()}>
            {creatingTeam ? "Creating…" : "Create team"}
          </Button>
        </form>
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
                  <Td align="right" mono muted={team.monthly_budget_mc === null}>
                    {team.monthly_budget_mc === null
                      ? "uncapped"
                      : formatUsd(team.monthly_budget_mc)}
                  </Td>
                  <Td muted>{formatRelative(team.created_at)}</Td>
                  <Td align="right">
                    <Button variant="danger" onClick={() => handleDeleteTeam(team)}>
                      Delete
                    </Button>
                  </Td>
                </tr>
              ))}
            </tbody>
          </TableShell>
        ))}
    </>
  );
}
