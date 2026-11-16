// Tests for the proposals model (ui/proposal_model.h): the state snapshot's
// proposals and proposalTimelines as the bridge sends them, and what the
// conversation's cards and the timeline's bands read back.

#include "proposal_model.h"
#include "test_registry.h"

#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QSignalSpy>
#include <QTest>

#include <cmath>
#include <initializer_list>

namespace kerf {

namespace {

QJsonObject agent(const char *kind, const char *name, const char *id, const char *role) {
    QJsonObject author{{"kind", kind}, {"name", name}, {"role", role}};
    if (id) author.insert("id", id);
    return author;
}

// The program at 25 fps, 30 s: the sea shot, the logo and its whoosh. With
// `logoAt` the logo (and the whoosh, 0.28 s after it) start there; `seaFx`
// gives the sea shot an effect stack.
QJsonObject timeline(double logoAt, const QJsonArray &seaFx = {}) {
    const auto clip = [](const char *id, const char *name, double start, double duration) {
        return QJsonObject{{"id", id}, {"name", name}, {"kind", "media"}, {"timelineStart", start}, {"duration", duration}};
    };
    QJsonObject sea = clip("sea", "B003_C011", 8.48, 4.72);
    if (!seaFx.isEmpty()) sea.insert("effects", seaFx);
    const double logoEnd = 30.0;
    return QJsonObject{
        {"fps", 25},
        {"tracks", QJsonArray{
                       QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{clip("road", "A012_C004", 0.0, 8.48), sea}}},
                       QJsonObject{{"id", "V4"}, {"type", "video"}, {"clips", QJsonArray{clip("logo", "Logo_reveal", logoAt, logoEnd - logoAt)}}},
                       QJsonObject{{"id", "A3"}, {"type", "audio"}, {"clips", QJsonArray{clip("whoosh", "Whoosh_logo", logoAt + 0.28, 1.0)}}},
                   }},
    };
}

QJsonArray hueFx() {
    return QJsonArray{QJsonObject{{"fx", "color.huehue"}, {"version", 1}, {"enabled", true}, {"params", QJsonObject{{"hue", -9}, {"sat", -6}}}}};
}

QJsonObject proposalEntry(const char *id, const char *status, const QJsonObject &author, int version,
                          const QJsonArray &versions, const char *script,
                          std::initializer_list<std::pair<const char *, QJsonValue>> extra = {}) {
    QJsonObject entry{{"proposalId", id},  {"status", status},   {"freshness", "current"}, {"baseRevision", 22},
                      {"script", script},  {"source", script},   {"documentDiff", ""},     {"author", author},
                      {"version", version}, {"supersedes", QJsonValue()}, {"versions", versions}};
    for (const auto &[key, value] : extra) entry.insert(key, value);
    return entry;
}

QJsonObject versionEntry(int version, const char *id, const char *script) {
    return QJsonObject{{"version", version}, {"proposalId", id}, {"baseRevision", 22}, {"script", script}, {"documentDiff", ""}};
}

// The snapshot at rev 22: two proposals wait (Claude Code's v2 and Kerf's
// colour key), two settled at rev 20.
QJsonObject seedSnapshot() {
    const QJsonObject claude = agent("mcp", "Claude Code", "claude-code", "MCP · via Kerf");
    const char *cc1 = "move logo to 00:00:25:21\nmove whoosh to 00:00:26:03\n";
    const char *cc2 = "move logo to 00:00:25:20\nmove whoosh to 00:00:26:02\n";
    const char *kg = "use main\neffect add sea color.huehue hue=-9 sat=-6\n";
    const QJsonArray proposals{
        proposalEntry("proposal-cc2", "pending", claude, 2,
                      QJsonArray{versionEntry(1, "proposal-cc1", cc1), versionEntry(2, "proposal-cc2", cc2)}, cc2,
                      {{"supersedes", "proposal-cc1"}, {"staged", "2026-11-03T09:15:00Z"}}),
        proposalEntry("proposal-kg", "pending", QJsonObject{{"kind", "kerf"}, {"name", "Kerf"}}, 1,
                      QJsonArray{versionEntry(1, "proposal-kg", kg)}, kg, {{"staged", "2026-11-02T10:01:00Z"}}),
        proposalEntry("proposal-cu", "accepted", agent("mcp", "Cursor", "cursor", "MCP"), 1,
                      QJsonArray{versionEntry(1, "proposal-cu", "move road to 00:00:00:00\n")}, "move road to 00:00:00:00\n",
                      {{"appliedRevision", 20}}),
        proposalEntry("proposal-cx", "accepted", agent("mcp", "Codex", "codex", "MCP"), 1,
                      QJsonArray{versionEntry(1, "proposal-cx", "trim end out -12f\n")}, "trim end out -12f\n",
                      {{"appliedRevision", 20}}),
    };
    const QJsonObject timelines{
        {"proposal-cc2", timeline(25.8)},
        {"proposal-cc1", timeline(25.84)},
        {"proposal-kg", timeline(26.12, hueFx())},
    };
    return QJsonObject{{"revision", 22},
                       {"timeline", timeline(26.12)},
                       {"proposals", proposals},
                       {"proposalTimelines", timelines}};
}

QStringList ids(const QList<ProposalModel::Proposal> &proposals) {
    QStringList out;
    for (const ProposalModel::Proposal &proposal : proposals) out << proposal.id;
    return out;
}

// The model as the editor holds it: the snapshot at rev 22, the playhead on 00:00:17:08.
void seed(ProposalModel &model) {
    model.setSnapshot(seedSnapshot());
    model.setPlayhead(17.32);
}

} // namespace

class ProposalModelTest final : public QObject {
    Q_OBJECT

private slots:
    void proposalRangesComeFromTheTimelineDiff() {
        ProposalModel model;
        seed(model);
        const ProposalModel::Proposal *logo = model.proposal(QStringLiteral("proposal-cc2"));
        QVERIFY(logo);
        QCOMPARE(logo->version, 2);
        QCOMPARE(logo->versions.size(), 2);
        QCOMPARE(logo->versions.first().proposalId, QStringLiteral("proposal-cc1"));
        QCOMPARE(logo->author.name, QStringLiteral("Claude Code"));
        // Its first line, as words for its card.
        QCOMPARE(logo->summary, QStringLiteral("move logo to 00:00:25:20"));
        QCOMPARE(logo->changes.size(), 2);
        QCOMPARE(logo->changes.at(0).clip, QStringLiteral("logo"));
        QCOMPARE(logo->changes.at(0).track, QStringLiteral("V4"));
        QVERIFY(logo->changes.at(0).moved && logo->changes.at(0).trimmed && !logo->changes.at(0).effects);
        QVERIFY(std::abs(logo->changes.at(0).shift() + 0.32) < 1e-9);
        QCOMPARE(logo->changes.at(1).clip, QStringLiteral("whoosh"));
        QVERIFY(logo->changes.at(1).moved && !logo->changes.at(1).trimmed);
        QVERIFY(logo->hasRange);
        QCOMPARE(logo->start, 25.8);
        QCOMPARE(logo->end, 30.0);

        const ProposalModel::Proposal *sea = model.proposal(QStringLiteral("proposal-kg"));
        QVERIFY(sea);
        QCOMPARE(sea->summary, QStringLiteral("effect add sea color.huehue hue=-9 sat=-6"));
        QCOMPARE(sea->changes.size(), 1);
        QVERIFY(sea->changes.first().effects && !sea->changes.first().moved);
        QCOMPARE(sea->start, 8.48);
        QCOMPARE(sea->end, 13.2);
        QVERIFY(std::abs(sea->distanceFrom(17.32) - 4.12) < 1e-9);
        QCOMPARE(sea->distanceFrom(10.0), 0.0);

        // Added and removed clips count, with their own spans.
        QJsonObject added = timeline(26.12);
        QJsonArray tracks = added["tracks"].toArray();
        QJsonObject v1 = tracks[0].toObject();
        QJsonArray clips = v1["clips"].toArray();
        clips.removeAt(0);
        clips.append(QJsonObject{{"id", "card"}, {"name", "Lower third"}, {"timelineStart", 14.0}, {"duration", 3.0}});
        v1["clips"] = clips;
        tracks[0] = v1;
        added["tracks"] = tracks;
        const QList<ProposalModel::ClipChange> changes = ProposalModel::diffTimelines(timeline(26.12), added);
        QCOMPARE(changes.size(), 2);
        QVERIFY(changes.at(0).added && changes.at(0).clip == QLatin1String("card"));
        QVERIFY(changes.at(1).removed && changes.at(1).clip == QLatin1String("road"));
        QCOMPARE(changes.at(1).oldEnd, 8.48);

        // A clip disabled, a clip retimed and a crossfade added are changes
        // too, with the spans they touch.
        const auto withTrack = [](QJsonObject timeline, int index, auto edit) {
            QJsonArray tracks = timeline["tracks"].toArray();
            QJsonObject track = tracks[index].toObject();
            edit(track);
            tracks[index] = track;
            timeline["tracks"] = tracks;
            return timeline;
        };
        const QJsonObject program = timeline(26.12);
        const QJsonObject disabled = withTrack(program, 1, [](QJsonObject &track) {
            QJsonArray clips = track["clips"].toArray();
            QJsonObject logoClip = clips[0].toObject();
            logoClip["enabled"] = false;
            clips[0] = logoClip;
            track["clips"] = clips;
        });
        QList<ProposalModel::ClipChange> off = ProposalModel::diffTimelines(program, disabled);
        QCOMPARE(off.size(), 1);
        QVERIFY(off.first().toggled && !off.first().moved && !off.first().properties);
        QCOMPARE(off.first().oldStart, 26.12);
        const QJsonObject reversed = withTrack(program, 0, [](QJsonObject &track) {
            QJsonArray clips = track["clips"].toArray();
            QJsonObject seaClip = clips[1].toObject();
            seaClip["reverse"] = true;
            clips[1] = seaClip;
            track["clips"] = clips;
        });
        off = ProposalModel::diffTimelines(program, reversed);
        QCOMPARE(off.size(), 1);
        QVERIFY(off.first().retimed && off.first().clip == QLatin1String("sea"));
        const QJsonObject faded = withTrack(program, 0, [](QJsonObject &track) {
            track["transitions"] = QJsonArray{QJsonObject{{"id", "x1"}, {"from", "road"}, {"to", "sea"}, {"fx", "cross"},
                                                          {"version", 1}, {"align", "center"}, {"start", 8.23},
                                                          {"end", 8.73}, {"duration", 0.5}}};
        });
        off = ProposalModel::diffTimelines(program, faded);
        QCOMPARE(off.size(), 1);
        QVERIFY(off.first().transition && off.first().added && off.first().clip == QLatin1String("x1"));
        QCOMPARE(off.first().track, QStringLiteral("V1"));
        QCOMPARE(off.first().newStart, 8.23);
        QCOMPARE(off.first().newEnd, 8.73);
        // Restyled (another fx) and removed.
        const QJsonObject restyled = withTrack(faded, 0, [](QJsonObject &track) {
            QJsonArray transitions = track["transitions"].toArray();
            QJsonObject fade = transitions[0].toObject();
            fade["fx"] = "dip";
            transitions[0] = fade;
            track["transitions"] = transitions;
        });
        off = ProposalModel::diffTimelines(faded, restyled);
        QCOMPARE(off.size(), 1);
        QVERIFY(off.first().transition && off.first().properties && !off.first().moved);
        off = ProposalModel::diffTimelines(faded, program);
        QCOMPARE(off.size(), 1);
        QVERIFY(off.first().transition && off.first().removed);
        QCOMPARE(off.first().oldEnd, 8.73);
    }

    // The open proposals pin nearest the playhead first: at 00:00:17:08 the
    // sea key (4.12 s away) before the logo (8.48 s); inside the logo's
    // range, the logo.
    void pinOrderAtThePlayhead() {
        ProposalModel model;
        seed(model);
        QCOMPARE(ids(model.openProposals()), (QStringList{"proposal-kg", "proposal-cc2"}));
        QCOMPARE(model.pinnedProposal(), QStringLiteral("proposal-kg"));
        QCOMPARE(model.openCount(), 2);
        QSignalSpy changed(&model, &ProposalModel::changed);
        model.setPlayhead(18.0); // still nearer the sea
        QCOMPARE(changed.count(), 0);
        model.setPlayhead(28.0); // inside the logo's range
        QCOMPARE(changed.count(), 1);
        QCOMPARE(model.pinnedProposal(), QStringLiteral("proposal-cc2"));
        // Found by any of its versions' ids.
        QVERIFY(model.isPinned(QStringLiteral("proposal-cc1")));
        QCOMPARE(model.proposal(QStringLiteral("proposal-cc1"))->id, QStringLiteral("proposal-cc2"));
        model.setPlayhead(28.0);
        QCOMPARE(changed.count(), 1);
    }

    // Accepting as kerfd does it: a state with the logo accepted at rev 23
    // and the sea key stale. The pin moves to the sea key, which keeps its
    // range against the new program.
    void acceptingMovesThePin() {
        ProposalModel model;
        seed(model);
        model.setPlayhead(28.0);
        QCOMPARE(model.pinnedProposal(), QStringLiteral("proposal-cc2"));
        QJsonObject snapshot = seedSnapshot();
        QJsonArray proposals = snapshot["proposals"].toArray();
        QJsonObject logo = proposals[0].toObject();
        logo["status"] = "accepted";
        logo["appliedRevision"] = 23;
        proposals[0] = logo;
        QJsonObject sea = proposals[1].toObject();
        sea["freshness"] = "stale";
        proposals[1] = sea;
        snapshot["proposals"] = proposals;
        snapshot["revision"] = 23;
        snapshot["timeline"] = timeline(25.8);
        snapshot["proposalTimelines"] = QJsonObject{{"proposal-kg", timeline(25.8, hueFx())}};
        QVERIFY(model.applyEvent(QStringLiteral("state"), QJsonObject{{"type", "state"}, {"snapshot", snapshot}}));
        QVERIFY(!model.applyEvent(QStringLiteral("comments"), QJsonObject{{"type", "comments"}}));
        QCOMPARE(ids(model.openProposals()), QStringList{"proposal-kg"});
        QCOMPARE(model.pinnedProposal(), QStringLiteral("proposal-kg"));
        QVERIFY(model.proposal(QStringLiteral("proposal-kg"))->stale);
        QCOMPARE(model.proposal(QStringLiteral("proposal-kg"))->changes.size(), 1);
        QCOMPARE(model.proposal(QStringLiteral("proposal-cc2"))->appliedRevision, qint64(23));
        QVERIFY(!model.proposal(QStringLiteral("proposal-cc2"))->isOpen());
    }

    // An older version's changes, for its card's Versions menu: v1 of the
    // logo moves it 7 frames earlier.
    void versionsDiffOnTheirOwn() {
        const QJsonObject snapshot = seedSnapshot();
        const QList<ProposalModel::ClipChange> v1 = ProposalModel::diffTimelines(
            snapshot.value("timeline").toObject(), snapshot.value("proposalTimelines").toObject().value("proposal-cc1").toObject());
        QCOMPARE(v1.size(), 2);
        QCOMPARE(v1.at(0).clip, QStringLiteral("logo"));
        QVERIFY(v1.at(0).moved);
        QCOMPARE(std::llround(v1.at(0).shift() * 25), -7LL);
        QCOMPARE(std::llround(v1.at(0).newStart * 25), 646LL);
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::ProposalModelTest)

#include "proposal_model_test.moc"
