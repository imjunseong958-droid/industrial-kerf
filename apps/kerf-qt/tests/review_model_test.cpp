// Tests for the review model (ui/review_model.h): the mockup's seed threads
// (mockup-spec 4.9) as the bridge sends them — comments {items}, the state
// snapshot's proposals and proposalTimelines, collab.status.user — and what
// the Comments panel and the ruler read back.

#include "review_model.h"
#include "test_registry.h"

#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QSettings>
#include <QSignalSpy>
#include <QTest>

#include <cmath>
#include <initializer_list>

namespace kerf {

namespace {

const QString kYou = QStringLiteral("ana@kerf.test");
const QString kMina = QStringLiteral("mina@kerf.test");
const QString kTom = QStringLiteral("tom@kerf.test");

QJsonObject person(const char *name, const QString &email, const char *role = nullptr) {
    QJsonObject author{{"kind", "person"}, {"name", name}, {"id", email}};
    if (role) author.insert("role", role);
    return author;
}

QJsonObject agent(const char *kind, const char *name, const char *id, const char *role) {
    QJsonObject author{{"kind", kind}, {"name", name}, {"role", role}};
    if (id) author.insert("id", id);
    return author;
}

QJsonObject mina() { return person("Mina Park", kMina, "director"); }
QJsonObject tom() { return person("Tom Lind", kTom, "colourist"); }
QJsonObject you() { return person("Ana Ruiz", kYou); }
QJsonObject kerfAuthor(const char *role = nullptr) {
    QJsonObject author{{"kind", "kerf"}, {"name", "Kerf"}};
    if (role) author.insert("role", role);
    return author;
}

QJsonObject note(const char *id, const QJsonObject &author, const char *created, const char *body,
                 std::initializer_list<std::pair<const char *, QJsonValue>> extra = {}) {
    QJsonObject record{{"id", id},           {"parent", QJsonValue()}, {"author", author}, {"body", body},
                       {"tags", QJsonArray{}}, {"blocking", false},    {"resolved", false}, {"created", created},
                       {"revision", 22}};
    for (const auto &[key, value] : extra) record.insert(key, value);
    return record;
}

QJsonObject reply(const char *id, const char *parent, const QJsonObject &author, const char *created, const char *body,
                  std::initializer_list<std::pair<const char *, QJsonValue>> extra = {}) {
    QJsonObject record = note(id, author, created, body, extra);
    record.insert("parent", parent);
    return record;
}

// The seed threads of mockup-spec 4.9 as kerfd's review log folds them,
// oldest first. The Kerf welcome is no record (setWelcomeShown). The two
// settled threads are as kerfd's accept leaves them (runtime.rs accept →
// review.rs resolve_proposal): the comments carrying the accepted proposal
// resolved at its applied revision (rev 20), by you when you accepted in
// the app, by nobody over MCP; Mina's root itself untouched.
QJsonArray seedComments() {
    const QJsonObject resolvedByYou = you();
    return QJsonArray{
        note("c-2916", agent("mcp", "Codex", "codex", "MCP"), "2026-11-01T12:00:00.000000Z",
             "End card holds a beat long, trimmed 12 frames",
             {{"at", 29.64}, {"frame", 741}, {"proposal", "proposal-cx"}, {"resolved", true}, {"resolvedRevision", 20}}),
        note("c-0300", mina(), "2026-11-02T09:00:00.000000Z", "Open on the road, not the sky",
             {{"at", 3.0}, {"frame", 75}}),
        reply("c-0300-r1", "c-0300", agent("mcp", "Cursor", "cursor", "MCP"), "2026-11-02T09:10:00.000000Z",
              "Moved A012_C004 to the head, trimmed 1 s of sky",
              {{"proposal", "proposal-cu"}, {"resolved", true}, {"resolvedBy", resolvedByYou}, {"resolvedRevision", 20}}),
        reply("c-0300-r2", "c-0300", mina(), "2026-11-02T09:20:00.000000Z", "Better, thanks"),
        note("c-0903", tom(), "2026-11-02T10:00:00.000000Z", "The sea reads magenta here",
             {{"at", 9.12}, {"frame", 228}, {"tags", QJsonArray{"color"}}}),
        reply("c-0903-r1", "c-0903", kerfAuthor(), "2026-11-02T10:01:00.000000Z", "Sea back toward cyan",
              {{"proposal", "proposal-kg"},
               {"steps", QJsonArray{"Sampled the sea on B003_C011 at 00:00:09:03", "Measured hue 9° off the cyan axis",
                                    "Drafted a hue-vs-hue key, sea only"}}}),
        note("c-1708", kerfAuthor("QC · captions"), "2026-11-03T08:00:00.000000Z",
             "Caption overlaps the lower-third by 8 px",
             {{"at", 17.32}, {"duration", 2.48}, {"frame", 433}, {"tags", QJsonArray{"#captions"}},
              {"qcKey", "qc:text.overlap:clip-12"}}),
        reply("c-1708-r1", "c-1708", tom(), "2026-11-03T08:30:00.000000Z", "Also clips the sun flare, circled",
              {{"drawing", QJsonArray{QJsonObject{{"tool", "ellipse"},
                                                  {"points", QJsonArray{QJsonArray{0.61, 0.22}, QJsonArray{0.74, 0.35}}},
                                                  {"color", "#ededed"}}}}}),
        note("c-2614", mina(), "2026-11-03T09:00:00.000000Z", "Logo lands late, should hit on the snare",
             {{"at", 26.56}, {"frame", 664}, {"blocking", true}}),
        reply("c-2614-r1", "c-2614", agent("mcp", "Claude Code", "claude-code", "MCP · via Kerf"),
              "2026-11-03T09:05:00.000000Z", "Move logo reveal to the snare", {{"proposal", "proposal-cc1"}}),
        reply("c-2614-r2", "c-2614", mina(), "2026-11-03T09:10:00.000000Z",
              "Two frames earlier, on the transient 00:00:26:12"),
        reply("c-2614-r3", "c-2614", agent("mcp", "Claude Code", "claude-code", "MCP · via Kerf"),
              "2026-11-03T09:15:00.000000Z", "Updated to v2, Logo_reveal now lands on 00:00:26:12",
              {{"proposal", "proposal-cc2"}}),
        note("c-2718", agent("qc", "QC", nullptr, "loudness"), "2026-11-03T10:00:00.000000Z",
             "True peak −0.4 dBTP, limit −1.0 dBTP (EBU R128)",
             {{"at", 27.72}, {"duration", 1.08}, {"frame", 693}, {"blocking", true}, {"tags", QJsonArray{"audio"}},
              {"qcKey", "qc:audio.true_peak"}}),
        reply("c-2718-r1", "c-2718", you(), "2026-11-03T10:05:00.000000Z", "@Kerf limit the music bus here"),
        reply("c-2718-r2", "c-2718", kerfAuthor(), "2026-11-03T10:06:00.000000Z",
              "A2 Music is locked. Unlock it, then ask again", {{"error", true}}),
        reply("c-2718-r3", "c-2718", you(), "2026-11-03T10:10:00.000000Z", "Unlocked A2, try again"),
    };
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
                       {"proposalTimelines", timelines},
                       {"reviewRevision", 41}};
}

QStringList ids(const QList<ReviewModel::Thread> &threads) {
    QStringList out;
    for (const ReviewModel::Thread &thread : threads) out << thread.id;
    return out;
}

QStringList ids(const QList<ReviewModel::Proposal> &proposals) {
    QStringList out;
    for (const ReviewModel::Proposal &proposal : proposals) out << proposal.id;
    return out;
}

QStringList stackIds(const QList<ReviewModel::MarkerStack> &stacks) {
    QStringList out;
    for (const ReviewModel::MarkerStack &stack : stacks) {
        QStringList group;
        for (const ReviewModel::LaneMarker &marker : stack.markers) group << marker.id;
        out << group.join(QLatin1Char('+'));
    }
    return out;
}

// The model as the editor holds it at the mockup's start: the seed, you,
// the welcome row shown, the playhead on 00:00:17:08.
void seed(ReviewModel &model) {
    model.setWelcomeShown(true);
    model.setIdentity(QJsonObject{{"name", "Ana Ruiz"}, {"email", kYou}});
    model.setSnapshot(seedSnapshot());
    model.setComments(seedComments());
    model.setPlayhead(17.32);
}

const QString kWelcome = QString::fromLatin1(ReviewModel::kWelcomeId);

} // namespace

class ReviewModelTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    // The chips read All 7, Open 4, Proposals 2; two notes block the master.
    void seedCountsMatchTheMockup() {
        ReviewModel model;
        seed(model);
        const ReviewModel::Counts counts = model.counts();
        QCOMPARE(counts.all, 7);
        QCOMPARE(counts.open, 4);
        QCOMPARE(counts.proposals, 2);
        QCOMPARE(counts.blocking, 2);
        // The welcome row is a thread only while it shows.
        model.setWelcomeShown(false);
        QCOMPARE(model.counts().all, 6);
        QCOMPARE(model.counts().open, 4);
    }

    // Replies sit under their root, oldest first; a proposal links its
    // thread through any of its versions' ids.
    void repliesGroupUnderTheirRoot() {
        ReviewModel model;
        seed(model);
        const ReviewModel::Thread *logo = model.thread(QStringLiteral("c-2614"));
        QVERIFY(logo);
        QCOMPARE(logo->origin, ReviewModel::Origin::Record);
        QCOMPARE(logo->replies.size(), 3);
        QCOMPARE(logo->replies.at(0).id, QStringLiteral("c-2614-r1"));
        QCOMPARE(logo->replies.at(2).id, QStringLiteral("c-2614-r3"));
        QCOMPARE(logo->openProposal, QStringLiteral("proposal-cc2"));
        QVERIFY(logo->open);
        QVERIFY(logo->blocking);
        QCOMPARE(model.threadOf(QStringLiteral("c-2614-r2")), logo);
        QCOMPARE(model.threadOf(QStringLiteral("proposal-cc1")), logo);
        QCOMPARE(model.proposal(QStringLiteral("proposal-cc1"))->id, QStringLiteral("proposal-cc2"));

        // Settled by accepting Cursor's proposal: its reply resolved at rev
        // 20, Mina's root never resolved.
        const ReviewModel::Thread *road = model.thread(QStringLiteral("c-0300"));
        QVERIFY(road && !road->open && road->resolved() && !road->root.resolved);
        QCOMPARE(road->replies.size(), 2);
        QCOMPARE(road->proposals, QStringList{QStringLiteral("proposal-cu")});
        QVERIFY(road->openProposal.isEmpty());
        QCOMPARE(road->acceptedProposal, QStringLiteral("proposal-cu"));
        QCOMPARE(road->acceptedRevision, qint64(20));
        const ReviewModel::Comment *moved = model.comment(QStringLiteral("c-0300-r1"));
        QCOMPARE(moved->resolvedRevision, qint64(20));
        QVERIFY(moved->hasResolvedBy);
        QCOMPARE(moved->resolvedBy.name, QStringLiteral("Ana Ruiz"));
        // Codex's note carries its proposal itself, so the accept resolved it.
        const ReviewModel::Thread *card = model.thread(QStringLiteral("c-2916"));
        QVERIFY(card && card->resolved() && card->root.resolved);
        QCOMPARE(card->acceptedRevision, qint64(20));
        // Open threads were settled by nothing.
        QCOMPARE(logo->acceptedRevision, qint64(-1));
        QVERIFY(logo->acceptedProposal.isEmpty());

        const ReviewModel::Comment *steps = model.comment(QStringLiteral("c-0903-r1"));
        QVERIFY(steps);
        QCOMPARE(steps->steps.size(), 3);
        QCOMPARE(model.comment(QStringLiteral("c-2718-r2"))->error, true);
        const ReviewModel::Comment *overlap = model.comment(QStringLiteral("c-1708"));
        QCOMPARE(overlap->tags, QStringList{QStringLiteral("captions")});
        QCOMPARE(overlap->frame, 433);
        QVERIFY(overlap->ranged());
        QCOMPARE(overlap->end(), 17.32 + 2.48);
        QVERIFY(model.thread(QStringLiteral("c-1708"))->hasDrawing);
        QVERIFY(!model.comment(QStringLiteral("c-1708"))->hasDrawing());

        // A reply whose root was deleted stands on its own.
        QJsonArray orphaned = seedComments();
        orphaned.append(reply("c-lost", "c-gone", tom(), "2026-11-03T11:00:00Z", "Still here"));
        model.setComments(orphaned);
        QVERIFY(model.thread(QStringLiteral("c-lost")));

        // kerfd writes microseconds and orders by (created, id); two replies
        // in one millisecond keep that order, not their ids'.
        QJsonArray close = seedComments();
        close.append(reply("c-1708-r9", "c-1708", tom(), "2026-11-03T11:00:00.000100Z", "First"));
        close.append(reply("c-1708-r2", "c-1708", mina(), "2026-11-03T11:00:00.000900Z", "Second"));
        model.setComments(close);
        const ReviewModel::Thread *overlapThread = model.thread(QStringLiteral("c-1708"));
        QCOMPARE(overlapThread->replies.size(), 3);
        QCOMPARE(overlapThread->replies.at(1).id, QStringLiteral("c-1708-r9"));
        QCOMPARE(overlapThread->replies.at(2).id, QStringLiteral("c-1708-r2"));
    }

    // All / Open / Proposals.
    void chipsFilterTheThreads() {
        ReviewModel model;
        seed(model);
        QCOMPARE(ids(model.threads()), (QStringList{kWelcome, "c-0300", "c-0903", "c-1708", "c-2614", "c-2718", "c-2916"}));
        QSignalSpy changed(&model, &ReviewModel::changed);
        QSignalSpy counts(&model, &ReviewModel::countsChanged);
        model.setChip(ReviewModel::Chip::Open);
        QCOMPARE(changed.count(), 1);
        QCOMPARE(ids(model.threads()), (QStringList{"c-0903", "c-1708", "c-2614", "c-2718"}));
        model.setChip(ReviewModel::Chip::Open);
        QCOMPARE(changed.count(), 1);
        model.setChip(ReviewModel::Chip::Proposals);
        QCOMPARE(ids(model.threads()), (QStringList{"c-0903", "c-2614"}));
        // Filters never move the counts.
        QCOMPARE(counts.count(), 0);
        QCOMPARE(model.counts().all, 7);
    }

    // From Kerf (Kerf and QC anywhere in the thread), Agents (MCP anywhere),
    // People (the note is a person's); Hide resolved; With drawings.
    void fromAndCheckFilters() {
        ReviewModel model;
        seed(model);
        model.setFrom(ReviewModel::From::Kerf);
        QCOMPARE(ids(model.threads()), (QStringList{kWelcome, "c-0903", "c-1708", "c-2718"}));
        model.setFrom(ReviewModel::From::Agents);
        QCOMPARE(ids(model.threads()), (QStringList{"c-0300", "c-2614", "c-2916"}));
        model.setFrom(ReviewModel::From::People);
        QCOMPARE(ids(model.threads()), (QStringList{"c-0300", "c-0903", "c-2614"}));
        QVERIFY(model.filtering());
        model.setChip(ReviewModel::Chip::Open);
        QCOMPARE(ids(model.threads()), (QStringList{"c-0903", "c-2614"}));
        model.clearFilters();
        QVERIFY(!model.filtering());
        QCOMPARE(model.chip(), ReviewModel::Chip::All);

        model.setHideResolved(true);
        QCOMPARE(ids(model.threads()), (QStringList{kWelcome, "c-0903", "c-1708", "c-2614", "c-2718"}));
        model.setHideResolved(false);
        model.setWithDrawings(true);
        QCOMPARE(ids(model.threads()), QStringList{"c-1708"});
    }

    // One tag at a time; the same tag again clears it; Blocking selects the
    // threads marked blocking. The search reads every comment of a thread.
    void tagAndSearchFilters() {
        ReviewModel model;
        seed(model);
        model.toggleTag(QStringLiteral("#color"));
        QCOMPARE(model.tag(), QStringLiteral("color"));
        QCOMPARE(ids(model.threads()), QStringList{"c-0903"});
        model.toggleTag(QStringLiteral("color"));
        QVERIFY(model.tag().isEmpty());
        QCOMPARE(model.threads().size(), 7);
        model.setTag(QStringLiteral("Blocking"));
        QCOMPARE(model.tag(), QString::fromLatin1(ReviewModel::kBlockingTag));
        QCOMPARE(ids(model.threads()), (QStringList{"c-2614", "c-2718"}));
        model.setTag(QString());

        model.setSearch(QStringLiteral("  magenta "));
        QCOMPARE(model.search(), QStringLiteral("magenta"));
        QCOMPARE(ids(model.threads()), QStringList{"c-0903"});
        model.setSearch(QStringLiteral("SUN FLARE"));
        QCOMPARE(ids(model.threads()), QStringList{"c-1708"});
        model.setSearch(QStringLiteral("colourist"));
        QCOMPARE(ids(model.threads()), (QStringList{"c-0903", "c-1708"}));
        model.setSearch(QStringLiteral("You"));
        QCOMPARE(ids(model.threads()), QStringList{"c-2718"});
        model.setSearch(QString());
        QCOMPARE(model.threads().size(), 7);
    }

    // #audio 1, #captions 1, #color 1, Blocking 2.
    void tagsCountTheirThreads() {
        ReviewModel model;
        seed(model);
        const QList<ReviewModel::TagCount> tags = model.tags();
        QCOMPARE(tags.size(), 4);
        const std::initializer_list<std::tuple<const char *, int, bool>> expected{
            {"audio", 1, false}, {"captions", 1, false}, {"color", 1, false}, {"blocking", 2, true}};
        int i = 0;
        for (const auto &[tag, count, blocking] : expected) {
            QCOMPARE(tags.at(i).tag, QString::fromLatin1(tag));
            QCOMPARE(tags.at(i).count, count);
            QCOMPARE(tags.at(i).blocking, blocking);
            ++i;
        }
        // kerfd's CSV writes the flag as the tag "blocking": read back, it is the flag.
        const ReviewModel::Comment read = ReviewModel::readComment(
            note("c-x", tom(), "2026-11-03T12:00:00Z", "x", {{"tags", QJsonArray{"#blocking", "#color", "color"}}}));
        QVERIFY(read.blocking);
        QCOMPARE(read.tags, QStringList{QStringLiteral("color")});
    }

    // Timecode (the default), Newest, Open first, Commenter; the welcome
    // stays on top.
    void sortOrders() {
        ReviewModel model;
        seed(model);
        QCOMPARE(model.sort(), ReviewModel::Sort::Timecode);
        QCOMPARE(ids(model.threads()), (QStringList{kWelcome, "c-0300", "c-0903", "c-1708", "c-2614", "c-2718", "c-2916"}));
        QSignalSpy changed(&model, &ReviewModel::changed);
        model.setSort(ReviewModel::Sort::Newest);
        QCOMPARE(changed.count(), 1);
        QCOMPARE(ids(model.threads()), (QStringList{kWelcome, "c-2718", "c-2614", "c-1708", "c-0903", "c-0300", "c-2916"}));
        model.setSort(ReviewModel::Sort::OpenFirst);
        QCOMPARE(ids(model.threads()), (QStringList{kWelcome, "c-0903", "c-1708", "c-2614", "c-2718", "c-0300", "c-2916"}));
        model.setSort(ReviewModel::Sort::Commenter);
        QCOMPARE(ids(model.threads()), (QStringList{kWelcome, "c-2916", "c-1708", "c-0300", "c-2614", "c-2718", "c-0903"}));
        // allThreads follows the sort too, and lookups stay right.
        QCOMPARE(model.allThreads().at(1).id, QStringLiteral("c-2916"));
        QCOMPARE(model.thread(QStringLiteral("c-0903"))->id, QStringLiteral("c-0903"));
        QCOMPARE(ReviewModel::sortName(ReviewModel::Sort::OpenFirst), QStringLiteral("Open first"));
        QCOMPARE(ReviewModel::sortName(ReviewModel::Sort::Timecode), QStringLiteral("Timecode"));
    }

    // Each proposal's range is the clips its timeline changes: Claude Code's
    // moves the logo and the whoosh, Kerf's keys the sea.
    void proposalRangesComeFromTheTimelineDiff() {
        ReviewModel model;
        seed(model);
        const ReviewModel::Proposal *logo = model.proposal(QStringLiteral("proposal-cc2"));
        QVERIFY(logo);
        QCOMPARE(logo->version, 2);
        QCOMPARE(logo->versions.size(), 2);
        QCOMPARE(logo->versions.first().proposalId, QStringLiteral("proposal-cc1"));
        QCOMPARE(logo->author.name, QStringLiteral("Claude Code"));
        QCOMPARE(logo->summary, QStringLiteral("Move logo reveal to the snare"));
        QCOMPARE(logo->threadId, QStringLiteral("c-2614"));
        QVERIFY(!logo->synthesised);
        QVERIFY(logo->blocking);
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

        const ReviewModel::Proposal *sea = model.proposal(QStringLiteral("proposal-kg"));
        QVERIFY(sea);
        QCOMPARE(sea->summary, QStringLiteral("Sea back toward cyan"));
        QCOMPARE(sea->changes.size(), 1);
        QVERIFY(sea->changes.first().effects && !sea->changes.first().moved);
        QCOMPARE(sea->start, 8.48);
        QCOMPARE(sea->end, 13.2);
        QVERIFY(!sea->blocking);
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
        const QList<ReviewModel::ClipChange> changes = ReviewModel::diffTimelines(timeline(26.12), added);
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
        QList<ReviewModel::ClipChange> off = ReviewModel::diffTimelines(program, disabled);
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
        off = ReviewModel::diffTimelines(program, reversed);
        QCOMPARE(off.size(), 1);
        QVERIFY(off.first().retimed && off.first().clip == QLatin1String("sea"));
        const QJsonObject faded = withTrack(program, 0, [](QJsonObject &track) {
            track["transitions"] = QJsonArray{QJsonObject{{"id", "x1"}, {"from", "road"}, {"to", "sea"}, {"fx", "cross"},
                                                          {"version", 1}, {"align", "center"}, {"start", 8.23},
                                                          {"end", 8.73}, {"duration", 0.5}}};
        });
        off = ReviewModel::diffTimelines(program, faded);
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
        off = ReviewModel::diffTimelines(faded, restyled);
        QCOMPARE(off.size(), 1);
        QVERIFY(off.first().transition && off.first().properties && !off.first().moved);
        off = ReviewModel::diffTimelines(faded, program);
        QCOMPARE(off.size(), 1);
        QVERIFY(off.first().transition && off.first().removed);
        QCOMPARE(off.first().oldEnd, 8.73);
    }

    // At 00:00:17:08 the blocking logo proposal pins before the nearer sea
    // key; with nothing blocking, the nearer one pins.
    void pinOrderAtThePlayhead() {
        ReviewModel model;
        seed(model);
        QCOMPARE(ids(model.openProposals()), (QStringList{"proposal-cc2", "proposal-kg"}));
        QCOMPARE(model.pinnedProposal(), QStringLiteral("proposal-cc2"));
        QVERIFY(model.isPinned(QStringLiteral("proposal-cc1")));
        QVERIFY(!model.isPinned(QStringLiteral("proposal-kg")));

        // Mina's note resolved without accepting: nothing blocks, and the
        // sea key (4.12 s away) is nearer than the logo (8.48 s).
        QJsonArray comments = seedComments();
        for (int i = 0; i < comments.size(); ++i) {
            QJsonObject record = comments[i].toObject();
            if (record["id"].toString() == QLatin1String("c-2614")) record["resolved"] = true;
            comments[i] = record;
        }
        model.setComments(comments);
        QCOMPARE(ids(model.openProposals()), (QStringList{"proposal-kg", "proposal-cc2"}));
        QSignalSpy changed(&model, &ReviewModel::changed);
        model.setPlayhead(18.0); // still nearer the sea
        QCOMPARE(changed.count(), 0);
        model.setPlayhead(28.0); // inside the logo's range
        QCOMPARE(changed.count(), 1);
        QCOMPARE(model.pinnedProposal(), QStringLiteral("proposal-cc2"));
        model.setPlayhead(28.0);
        QCOMPARE(changed.count(), 1);
    }

    // Accepting the pinned proposal as kerfd does it: first a state with it
    // accepted at rev 23 and the other stale, then the comments with the two
    // replies carrying it (cc1 and cc2) resolved at rev 23 by you, Mina's
    // root untouched. The pin moves to the sea key; the thread is settled
    // ("Accepted in rev 23") and the counts drop.
    void acceptingMovesThePin() {
        ReviewModel model;
        seed(model);
        QSignalSpy counts(&model, &ReviewModel::countsChanged);

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
        model.applyEvent(QStringLiteral("state"), QJsonObject{{"type", "state"}, {"snapshot", snapshot}});
        // Until the review log's resolves arrive, the thread stays open.
        QVERIFY(model.thread(QStringLiteral("c-2614"))->open);
        QCOMPARE(model.counts().open, 4);
        QCOMPARE(model.counts().blocking, 2);
        QCOMPARE(model.counts().proposals, 1);

        const auto acceptedLog = [] {
            QJsonArray comments = seedComments();
            for (int i = 0; i < comments.size(); ++i) {
                QJsonObject record = comments[i].toObject();
                const QString id = record["id"].toString();
                // The two replies whose `proposal` names cc1 and cc2.
                if (id == QLatin1String("c-2614-r1") || id == QLatin1String("c-2614-r3")) {
                    record["resolved"] = true;
                    record["resolvedRevision"] = 23;
                    record["resolvedBy"] = you();
                }
                comments[i] = record;
            }
            return comments;
        };
        const QJsonArray comments = acceptedLog();
        model.applyEvent(QStringLiteral("comments"), QJsonObject{{"type", "comments"}, {"items", comments}, {"reviewRevision", 42}});

        QCOMPARE(ids(model.openProposals()), QStringList{"proposal-kg"});
        QCOMPARE(model.pinnedProposal(), QStringLiteral("proposal-kg"));
        QVERIFY(model.proposal(QStringLiteral("proposal-kg"))->stale);
        // The sea key's range is still its own clip, against the new program.
        QCOMPARE(model.proposal(QStringLiteral("proposal-kg"))->changes.size(), 1);
        QCOMPARE(model.proposal(QStringLiteral("proposal-cc2"))->appliedRevision, qint64(23));
        const ReviewModel::Counts after = model.counts();
        QCOMPARE(after.open, 3);
        QCOMPARE(after.proposals, 1);
        QCOMPARE(after.blocking, 1);
        QVERIFY(counts.count() >= 1);
        const ReviewModel::Thread *thread = model.thread(QStringLiteral("c-2614"));
        QVERIFY(thread->openProposal.isEmpty());
        QCOMPARE(thread->proposals, QStringList{QStringLiteral("proposal-cc2")});
        QVERIFY(!thread->root.resolved);
        QVERIFY(!thread->open && !thread->blocking && thread->resolved());
        QVERIFY(thread->flagged); // still listed under Blocking
        QCOMPARE(thread->acceptedProposal, QStringLiteral("proposal-cc2"));
        QCOMPARE(thread->acceptedRevision, qint64(23));
        QVERIFY(model.laneMarkers().at(3).id == QLatin1String("c-2614") && model.laneMarkers().at(3).resolved);
        model.setHideResolved(true);
        QVERIFY(!ids(model.threads()).contains(QStringLiteral("c-2614")));
        model.setHideResolved(false);
        model.setChip(ReviewModel::Chip::Proposals);
        QCOMPARE(ids(model.threads()), QStringList{"c-0903"});
        model.setChip(ReviewModel::Chip::All);

        // Reopening a reply that carried it reopens the thread.
        QJsonArray reopened = comments;
        for (int i = 0; i < reopened.size(); ++i) {
            QJsonObject record = reopened[i].toObject();
            if (record["id"].toString() == QLatin1String("c-2614-r3")) {
                record["resolved"] = false;
                record.remove("resolvedRevision");
                record.remove("resolvedBy");
            }
            reopened[i] = record;
        }
        model.setComments(reopened);
        QVERIFY(model.thread(QStringLiteral("c-2614"))->open);
        QCOMPARE(model.counts().blocking, 2);
        // Resolves at some other revision were not the accept's.
        QJsonArray elsewhere = acceptedLog();
        for (int i = 0; i < elsewhere.size(); ++i) {
            QJsonObject record = elsewhere[i].toObject();
            if (record["resolvedRevision"].toInt() == 23) record["resolvedRevision"] = 22;
            elsewhere[i] = record;
        }
        model.setComments(elsewhere);
        QVERIFY(model.thread(QStringLiteral("c-2614"))->open);
        QCOMPARE(model.thread(QStringLiteral("c-2614"))->acceptedRevision, qint64(-1));
    }

    // A proposal no comment links to still gets a thread, from its author.
    void unlinkedProposalGetsAThread() {
        ReviewModel model;
        seed(model);
        QJsonObject snapshot = seedSnapshot();
        QJsonArray proposals = snapshot["proposals"].toArray();
        const char *script = "use main\ntrim road out -10f\n";
        proposals.prepend(proposalEntry("proposal-cu9", "pending", agent("mcp", "Cursor", "cursor", "MCP"), 1,
                                        QJsonArray{versionEntry(1, "proposal-cu9", script)}, script,
                                        {{"staged", "2026-11-03T11:00:00Z"}}));
        snapshot["proposals"] = proposals;
        QJsonObject timelines = snapshot["proposalTimelines"].toObject();
        QJsonObject trimmed = timeline(26.12);
        QJsonArray tracks = trimmed["tracks"].toArray();
        QJsonObject v1 = tracks[0].toObject();
        QJsonArray clips = v1["clips"].toArray();
        QJsonObject road = clips[0].toObject();
        road["duration"] = 8.08;
        clips[0] = road;
        v1["clips"] = clips;
        tracks[0] = v1;
        trimmed["tracks"] = tracks;
        timelines["proposal-cu9"] = trimmed;
        snapshot["proposalTimelines"] = timelines;
        model.setSnapshot(snapshot);

        const QString id = QStringLiteral("proposal:proposal-cu9");
        const ReviewModel::Thread *thread = model.thread(id);
        QVERIFY(thread);
        QCOMPARE(thread->origin, ReviewModel::Origin::Proposal);
        QCOMPARE(thread->root.author.name, QStringLiteral("Cursor"));
        QCOMPARE(thread->root.author.kind, QStringLiteral("mcp"));
        QCOMPARE(thread->root.body, QStringLiteral("trim road out -10f"));
        QCOMPARE(thread->openProposal, QStringLiteral("proposal-cu9"));
        QVERIFY(thread->open);
        QCOMPARE(model.threadOf(QStringLiteral("proposal-cu9")), thread);
        QVERIFY(model.proposal(QStringLiteral("proposal-cu9"))->synthesised);
        QCOMPARE(model.counts().all, 8);
        QCOMPARE(model.counts().open, 5);
        QCOMPARE(model.counts().proposals, 3);
        // It sorts by its range (the road, from 0 s), and has no lane mark.
        QCOMPARE(ids(model.threads()).mid(0, 3), (QStringList{kWelcome, id, "c-0300"}));
        for (const ReviewModel::LaneMarker &marker : model.laneMarkers()) QVERIFY(marker.id != id);
        model.setFrom(ReviewModel::From::Agents);
        QVERIFY(ids(model.threads()).contains(id));
    }

    // One mark per timed root, by time, with its look.
    void laneMarkersCarryKindAndState() {
        ReviewModel model;
        seed(model);
        const QList<ReviewModel::LaneMarker> markers = model.laneMarkers();
        QCOMPARE(markers.size(), 6);
        const QStringList order{"c-0300", "c-0903", "c-1708", "c-2614", "c-2718", "c-2916"};
        const QStringList initials{"MP", "TL", "K", "MP", "Q", "Co"};
        for (int i = 0; i < markers.size(); ++i) {
            QCOMPARE(markers.at(i).id, order.at(i));
            QCOMPARE(markers.at(i).initials, initials.at(i));
            QVERIFY(!markers.at(i).current);
        }
        QVERIFY(markers.at(0).circle && !markers.at(0).glyph && markers.at(0).resolved);
        QVERIFY(markers.at(2).glyph && !markers.at(2).circle);
        QVERIFY(markers.at(2).ranged);
        QCOMPARE(markers.at(2).end, 17.32 + 2.48);
        QCOMPARE(markers.at(4).kind, QStringLiteral("qc"));
        QVERIFY(markers.at(4).ranged);
        QVERIFY(!markers.at(3).ranged);
        QVERIFY(!markers.at(5).circle && !markers.at(5).glyph && markers.at(5).resolved);
        QCOMPARE(markers.at(1).role, QStringLiteral("colourist"));

        // Initials: two words give two capitals, one word its first two letters.
        QCOMPARE(ReviewModel::initials({QStringLiteral("mcp"), QStringLiteral("Claude Code"), {}, {}, {}}), QStringLiteral("CC"));
        QCOMPARE(ReviewModel::initials({QStringLiteral("mcp"), QStringLiteral("Cursor"), {}, {}, {}}), QStringLiteral("Cu"));
        // Initials the author record carries win (the mockup's "Cx").
        const ReviewModel::Author codex = ReviewModel::readAuthor(
            QJsonObject{{"kind", "mcp"}, {"name", "Codex"}, {"id", "codex"}, {"initials", "Cx"}});
        QCOMPARE(ReviewModel::initials(codex), QStringLiteral("Cx"));
    }

    // Marks closer than 20 px at the lane's width make one count stack; a
    // chain of near marks is one stack.
    void markerStacksAtAWidth() {
        ReviewModel model;
        seed(model);
        QCOMPARE(stackIds(model.markerStacks(600, 30)), (QStringList{"c-0300", "c-0903", "c-1708", "c-2614", "c-2718", "c-2916"}));
        // 26.56 s and 27.72 s land 15.5 px apart; 29.64 s clamps to 390, 20.4 px on.
        const QList<ReviewModel::MarkerStack> at400 = model.markerStacks(400, 30);
        QCOMPARE(stackIds(at400), (QStringList{"c-0300", "c-0903", "c-1708", "c-2614+c-2718", "c-2916"}));
        QVERIFY(at400.at(3).isStack());
        QVERIFY(!at400.at(4).isStack());
        QVERIFY(std::abs(at400.at(3).x - 26.56 / 30 * 400) < 1e-9);
        QCOMPARE(at400.at(4).x, 390.0);
        QCOMPARE(at400.at(0).x, 40.0);
        QCOMPARE(stackIds(model.markerStacks(300, 30)), (QStringList{"c-0300", "c-0903", "c-1708", "c-2614+c-2718+c-2916"}));
    }

    // Setting the current thread from any comment in it, or its proposal.
    void currentThread() {
        ReviewModel model;
        seed(model);
        QSignalSpy current(&model, &ReviewModel::currentChanged);
        model.setCurrentThread(QStringLiteral("c-1708-r1"));
        QCOMPARE(model.currentThread(), QStringLiteral("c-1708"));
        QCOMPARE(current.count(), 1);
        QCOMPARE(current.first().first().toString(), QStringLiteral("c-1708"));
        model.setCurrentThread(QStringLiteral("c-1708"));
        QCOMPARE(current.count(), 1);
        QVERIFY(model.laneMarkers().at(2).current);
        model.setCurrentThread(QStringLiteral("proposal-cc1"));
        QCOMPARE(model.currentThread(), QStringLiteral("c-2614"));
        model.setCurrentThread(QStringLiteral("no-such-comment"));
        QCOMPARE(model.currentThread(), QStringLiteral("c-2614"));

        // Gone from the log: cleared.
        QJsonArray comments;
        for (const QJsonValue &value : seedComments())
            if (!value.toObject()["id"].toString().startsWith(QLatin1String("c-2614"))) comments.append(value);
        model.setComments(comments);
        QVERIFY(model.currentThread().isEmpty());
        QCOMPARE(current.last().first().toString(), QString());

        QCOMPARE(model.threadAt(17.5), QStringLiteral("c-1708"));
        QCOMPARE(model.threadAt(9.2, 0.1), QStringLiteral("c-0903"));
        QVERIFY(model.threadAt(5.0).isEmpty());
    }

    // You are the person whose id is collab's user email; View as another
    // person is guest mode.
    void identityAndViewAs() {
        ReviewModel model;
        model.setComments(seedComments());
        QVERIFY(!model.isOwn(*model.comment(QStringLiteral("c-2718-r1"))));
        model.applyEvent(QStringLiteral("collab"),
                         QJsonObject{{"status", QJsonObject{{"branch", "main"}, {"user", QJsonObject{{"name", "Ana Ruiz"}, {"email", "ANA@kerf.test"}}}}}});
        QCOMPARE(model.identityName(), QStringLiteral("Ana Ruiz"));
        const ReviewModel::Comment *mine = model.comment(QStringLiteral("c-2718-r1"));
        QVERIFY(model.isOwn(*mine));
        QVERIFY(model.isOwn(mine->json));
        QCOMPARE(model.displayName(mine->author), QStringLiteral("You"));
        QVERIFY(!model.isOwn(*model.comment(QStringLiteral("c-2614"))));
        QVERIFY(!model.isOwn(*model.comment(QStringLiteral("c-2718-r2")))); // Kerf
        QVERIFY(model.laneMarkers().at(0).name == QLatin1String("Mina Park"));

        const QList<ReviewModel::Person> people = model.people();
        QCOMPARE(people.size(), 2);
        QCOMPARE(people.at(0).name, QStringLiteral("Mina Park"));
        QCOMPARE(people.at(1).id, kTom);

        QVERIFY(!model.guestMode());
        QSignalSpy changed(&model, &ReviewModel::changed);
        model.setViewAs(kMina);
        QVERIFY(model.guestMode());
        QCOMPARE(model.viewAsName(), QStringLiteral("Mina Park"));
        QCOMPARE(changed.count(), 1);
        model.setViewAs(kYou);
        QVERIFY(!model.guestMode());
        model.setViewAs(QString());
        QVERIFY(!model.guestMode());
        QVERIFY(model.viewAsName().isEmpty());

        // A collab status with no user (null) forgets who you are.
        model.applyEvent(QStringLiteral("collab"), QJsonObject{{"status", QJsonObject{{"user", QJsonValue()}}}});
        QVERIFY(!model.isOwn(*model.comment(QStringLiteral("c-2718-r1"))));
    }

    // The bridge's events reach the model through applyEvent.
    void eventsFeedTheModel() {
        ReviewModel model;
        QVERIFY(!model.applyEvent(QStringLiteral("clock"), QJsonObject{{"position", 1.0}}));
        QVERIFY(!model.applyEvent(QStringLiteral("collab"), QJsonObject{{"status", QJsonObject{{"branch", "main"}}}}));
        QSignalSpy counts(&model, &ReviewModel::countsChanged);
        QVERIFY(model.applyEvent(QStringLiteral("state"), QJsonObject{{"snapshot", seedSnapshot()}}));
        QCOMPARE(model.counts().proposals, 2);
        // Before the comments arrive, both proposals stand as their own threads.
        QCOMPARE(model.counts().all, 2);
        QVERIFY(model.applyEvent(QStringLiteral("comments"), QJsonObject{{"items", seedComments()}, {"reviewRevision", 41}}));
        QCOMPARE(model.counts().all, 6);
        QCOMPARE(model.counts().open, 4);
        QCOMPARE(counts.count(), 2);
        // The same snapshot again changes nothing.
        QSignalSpy changed(&model, &ReviewModel::changed);
        model.applyEvent(QStringLiteral("state"), QJsonObject{{"snapshot", seedSnapshot()}});
        model.applyEvent(QStringLiteral("comments"), QJsonObject{{"items", seedComments()}});
        QCOMPARE(changed.count(), 0);
    }

    // What the Comments panel builds on: the welcome passes All and From
    // Kerf only (no chip, tag or search keeps a row with no record), and an
    // older version's proposal timeline diffs on its own (the versions
    // menu's "v1 00:00:25:21 · −7 fr").
    void welcomeAndVersionsForThePanel() {
        ReviewModel model;
        seed(model);
        const auto shown = [&model] {
            QStringList out;
            for (const ReviewModel::Thread &thread : model.threads()) out << thread.id;
            return out;
        };
        QCOMPARE(shown().constFirst(), kWelcome);
        model.setFrom(ReviewModel::From::Kerf);
        QCOMPARE(shown().constFirst(), kWelcome);
        model.setFrom(ReviewModel::From::Anyone);
        model.setChip(ReviewModel::Chip::Open);
        QVERIFY(!shown().contains(kWelcome));
        model.setChip(ReviewModel::Chip::All);
        model.setSearch(QStringLiteral("magenta"));
        QCOMPARE(shown(), QStringList{QStringLiteral("c-0903")});
        model.setSearch(QString());
        // v1 of Claude Code's proposal moves the logo 7 frames earlier.
        const QJsonObject snapshot = seedSnapshot();
        const QList<ReviewModel::ClipChange> v1 = ReviewModel::diffTimelines(
            snapshot.value("timeline").toObject(), snapshot.value("proposalTimelines").toObject().value("proposal-cc1").toObject());
        QCOMPARE(v1.size(), 2);
        QCOMPARE(v1.at(0).clip, QStringLiteral("logo"));
        QVERIFY(v1.at(0).moved);
        QCOMPARE(std::llround(v1.at(0).shift() * 25), -7LL);
        QCOMPARE(std::llround(v1.at(0).newStart * 25), 646LL);
        // The pin is the current version, found by any of its versions' ids.
        QCOMPARE(model.proposal(QStringLiteral("proposal-cc1"))->id, QStringLiteral("proposal-cc2"));
        QVERIFY(model.isPinned(QStringLiteral("proposal-cc1")));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::ReviewModelTest)

#include "review_model_test.moc"
