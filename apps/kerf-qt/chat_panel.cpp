#include "chat_panel.h"

#include "components.h"
#include "composer.h"
#include "design_tokens.h"
#include "icons.h"
#include "proposal_card.h"

#include <QApplication>
#include <QClipboard>
#include <QCoreApplication>
#include <QEnterEvent>
#include <QFontMetricsF>
#include <QFrame>
#include <QHBoxLayout>
#include <QKeyEvent>
#include <QLabel>
#include <QLineEdit>
#include <QMenu>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPushButton>
#include <QScrollBar>
#include <QShortcut>
#include <QSignalBlocker>
#include <QTextEdit>
#include <QTimer>
#include <QVBoxLayout>
#include <QVariantAnimation>

#include <algorithm>
#include <cmath>
#include <limits>
#include <utility>

namespace kerf {

using namespace tokens;

namespace {

QLabel *makeLabel(const TypeStyle &style, QRgb ink, QWidget *parent = nullptr) {
    auto *label = new QLabel(parent);
    label->setFont(tokens::font(style));
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, color(ink));
    label->setPalette(palette);
    label->setFixedHeight(style.line);
    return label;
}

// A one-line label that ends in an ellipsis where it is cut (.pt0).
class ElidedLabel final : public QLabel {
public:
    using QLabel::QLabel;

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setFont(font());
        p.setPen(palette().color(QPalette::WindowText));
        const QFontMetricsF metrics(font());
        const QString shown = metrics.elidedText(text(), Qt::ElideRight, width());
        p.drawText(QPointF(0, (height() - metrics.height()) / 2 + metrics.ascent()), shown);
    }
};

QJsonObject findClip(const QJsonObject &timeline, const QString &clipId) {
    for (const QJsonValue &track : timeline.value(QStringLiteral("tracks")).toArray())
        for (const QJsonValue &clip : track.toObject().value(QStringLiteral("clips")).toArray())
            if (clip.toObject().value(QStringLiteral("id")).toString() == clipId) return clip.toObject();
    return {};
}

// A colour effect: what the vectorscope chip can show.
bool isColourEffect(const QString &fx) {
    const QString id = fx.toLower();
    return id.startsWith(QLatin1String("color.")) || id.startsWith(QLatin1String("colour.")) || id.contains(QLatin1String("hue")) ||
           id.contains(QLatin1String("grade")) || id.contains(QLatin1String("lut")) || id.contains(QLatin1String("look")) ||
           id.contains(QLatin1String("curve")) || id.contains(QLatin1String("balance")) || id.contains(QLatin1String("saturation"));
}

// "color.huehue" → "Huehue", "color.white_balance" → "White balance".
QString effectLabel(const QString &fx) {
    QString name = fx.section(QLatin1Char('.'), -1);
    name.replace(QLatin1Char('_'), QLatin1Char(' '));
    return name.isEmpty() ? fx : name.left(1).toUpper() + name.mid(1);
}

// A parameter's value as the card says it: "−9", "0.5"; empty for keyframes.
QString paramValue(const QJsonValue &value) {
    if (value.isDouble()) {
        const double number = value.toDouble();
        const QString magnitude = QString::number(std::abs(number), 'g', 4);
        return number < 0 ? QStringLiteral("−") + magnitude : magnitude;
    }
    if (value.isBool()) return value.toBool() ? QStringLiteral("on") : QStringLiteral("off");
    if (value.isString()) return value.toString();
    return {};
}

// The static parameters only (keyframed ones cannot be previewed as one value).
QJsonObject staticParams(const QJsonObject &params) {
    QJsonObject out;
    for (auto it = params.begin(); it != params.end(); ++it)
        if (!it.value().isArray() && !it.value().isObject()) out.insert(it.key(), it.value());
    return out;
}

QString firstName(const QString &name) { return name.section(QLatin1Char(' '), 0, 0, QString::SectionSkipEmpty); }

} // namespace

// ------------------------------------------------------------------ ChatIconButton

ChatIconButton::ChatIconButton(Glyph glyph, int diameter, QWidget *parent)
    : QAbstractButton(parent), m_glyph(glyph), m_diameter(diameter) {
    setFixedSize(diameter, diameter);
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
}

QSize ChatIconButton::sizeHint() const { return QSize(m_diameter, m_diameter); }

void ChatIconButton::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const bool hover = isEnabled() && (underMouse() || isDown());
    if (hover) {
        p.setPen(Qt::NoPen);
        p.setBrush(color(raised));
        p.drawRoundedRect(QRectF(rect()), radius::control, radius::control);
    }
    const QColor ink = !isEnabled() ? color(dis) : hover ? color(tokens::ink) : color(ink2);
    const QString icon = m_glyph == Glyph::Close ? QStringLiteral("g-close") : QStringLiteral("g-more");
    icons::paint(p, icon, QRectF(rect()), ink, m_diameter <= 20 ? 14 : 16);
    if (hasFocus()) ui::paintFocusRing(p, QRectF(rect()), radius::control);
}

// ------------------------------------------------------------------ ChatPanel

ChatPanel::ChatPanel(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("chatPanel"));
    setAttribute(Qt::WA_StyledBackground, false);
    setAccessibleName(QStringLiteral("Comments"));
    m_welcomeSince = QDateTime::currentDateTime();

    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    layout->addWidget(buildHeader());
    layout->addWidget(buildSearch());
    layout->addWidget(buildPinBar());

    m_list = new ui::FadeScrollArea;
    m_list->setObjectName(QStringLiteral("chatScroll"));
    m_list->setWidgetResizable(true);
    m_list->setFrameShape(QFrame::NoFrame);
    m_list->setFadeColor(color(tokens::chrome));
    m_list->setStyleSheet(QStringLiteral("QScrollArea#chatScroll { background: transparent; border: 0; }"));
    m_list->viewport()->setAutoFillBackground(false);
    m_content = new QWidget;
    m_content->setObjectName(QStringLiteral("chatContent"));
    m_content->setAutoFillBackground(false);
    m_listLayout = new QVBoxLayout(m_content);
    // pad 12 0 16; a row's 6 bleed is part of the 12.
    m_listLayout->setContentsMargins(0, kListTop - CommentRow::kBleedY, 0, kListBottom - CommentRow::kBleedY);
    // A thread's line sits 12 under the content above it (6 of it the bleed).
    m_listLayout->setSpacing(12 - CommentRow::kBleedY);
    m_listLayout->addStretch(1);
    m_list->setWidget(m_content);
    layout->addWidget(m_list, 1);
    connect(m_list->verticalScrollBar(), &QScrollBar::valueChanged, this, [this](int value) {
        m_follow = value >= m_list->verticalScrollBar()->maximum() - 4;
        updatePinBar();
    });
    connect(m_list->verticalScrollBar(), &QScrollBar::rangeChanged, this, [this] { updatePinBar(); });

    m_replyBar = new ReplyBar;
    connect(m_replyBar, &ReplyBar::cancelled, this, [this] {
        m_replyTarget.clear();
        if (!m_editing.isEmpty()) {
            m_editing.clear();
            m_composer->clear();
        }
    });
    layout->addWidget(m_replyBar);
    m_composer = new Composer;
    m_composer->setOverlayParent(this);
    connect(m_composer, &Composer::submitRequested, this, &ChatPanel::postRequested);
    connect(m_composer, &Composer::stopRequested, this, &ChatPanel::stopRequested);
    connect(m_composer, &Composer::drawClicked, this, [this] {
        if (m_composer->drawState() == Composer::Draw::None) emit drawRequested();
        else if (m_composer->drawState() == Composer::Draw::Drawing) emit drawCancelRequested();
        else m_composer->setDraw(Composer::Draw::None);
    });
    connect(m_composer, &Composer::suggestionChosen, this, [this](int index) {
        if (index < 0 || index >= m_suggestionList.size()) return;
        const Suggestion suggestion = m_suggestionList.at(index);
        if (suggestion.run) emit runRequested(suggestion.request);
        else m_composer->fill(suggestion.request);
    });
    connect(m_composer, &Composer::escapePressed, this, [this] {
        if (m_replyBar->isOpen()) {
            m_replyBar->setTarget({}, {});
            emit m_replyBar->cancelled();
        } else if (m_composer->isBusy()) {
            emit stopRequested();
        } else if (m_composer->drawState() == Composer::Draw::Drawing) {
            emit drawCancelRequested();
        } else {
            emit escapeUnused();
        }
    });
    layout->addWidget(m_composer);

    m_stash = new QWidget(this);
    m_stash->hide();

    // The Kerf welcome (no record): who Kerf is, and what to try.
    m_welcomeRow = new CommentRow(CommentRow::Kind::Kerf,
                                  QStringLiteral("Mention @Kerf on any note, or describe an edit below. Each change comes back "
                                                 "as a proposal you preview before it touches the cut"),
                                  m_welcomeSince, m_stash);
    m_welcomeRow->setObjectName(QStringLiteral("welcomeRow"));
    m_welcomeRow->setRole(QStringLiteral("agent"));
    ui::setTip(m_welcomeRow->timeLabel(), QStringLiteral("Since this window opened"));
    m_welcomeTime = m_welcomeRow->timeLabel();
    connectRow(m_welcomeRow);

    m_clock = new QTimer(this);
    m_clock->setInterval(30 * 1000);
    connect(m_clock, &QTimer::timeout, this, &ChatPanel::refreshTimes);
    m_clock->start();

    auto *find = new QShortcut(QKeySequence::Find, this);
    find->setContext(Qt::WidgetWithChildrenShortcut);
    connect(find, &QShortcut::activated, this, [this] { setSearchShown(true); });

    rebuild();
    updateHeader();
    updateSuggestions();
}

ChatPanel::~ChatPanel() = default;

void ChatPanel::focusComposer() { input()->setFocus(Qt::ShortcutFocusReason); }

QWidget *ChatPanel::buildHeader() {
    auto *header = new QWidget;
    header->setObjectName(QStringLiteral("chatHeader"));
    header->setFixedHeight(kHeaderHeight);
    auto *layout = new QHBoxLayout(header);
    layout->setContentsMargins(16, 0, 12, 0);
    layout->setSpacing(4);
    m_title = makeLabel(tokens::title, ink);
    m_title->setObjectName(QStringLiteral("chatTitle"));
    m_title->setText(QStringLiteral("Comments"));
    layout->addWidget(m_title);
    layout->addStretch(1);
    m_more = new ChatIconButton(ChatIconButton::Glyph::More, 24);
    m_more->setObjectName(QStringLiteral("chatMore"));
    ui::setTip(m_more, QStringLiteral("Search, sort, filter and more"));
    m_more->setAccessibleName(QStringLiteral("More"));
    connect(m_more, &QAbstractButton::clicked, this, [this] { ui::popupMenu(buildMoreMenu(), m_more); });
    // On while a filter hides rows.
    m_filterDot = new ui::Dot(color(ink), m_more);
    m_filterDot->setGeometry(24 - 3 - 6, 3, 6, 6);
    m_filterDot->setAttribute(Qt::WA_TransparentForMouseEvents);
    m_filterDot->hide();
    layout->addWidget(m_more);
    return header;
}

QWidget *ChatPanel::buildSearch() {
    m_searchRow = new QWidget;
    m_searchRow->setObjectName(QStringLiteral("chatSearch"));
    m_searchRow->setFixedHeight(kSearchHeight);
    auto *layout = new QHBoxLayout(m_searchRow);
    layout->setContentsMargins(16, 0, 12, 8);
    layout->setSpacing(4);
    m_search = new QLineEdit;
    m_search->setObjectName(QStringLiteral("commentSearch"));
    m_search->setPlaceholderText(QStringLiteral("Search comments"));
    m_search->setClearButtonEnabled(true);
    m_search->setFont(tokens::font(control));
    m_search->setStyleSheet(QStringLiteral("QLineEdit#commentSearch { background: %1; border: 0; border-radius: 6px; padding: 0 8px; "
                                           "min-height: 24px; max-height: 24px; color: %2; }")
                                .arg(color(raised).name(QColor::HexRgb), color(ink).name(QColor::HexRgb)));
    {
        QPalette palette = m_search->palette();
        palette.setColor(QPalette::PlaceholderText, color(ink3));
        m_search->setPalette(palette);
    }
    m_search->installEventFilter(this);
    connect(m_search, &QLineEdit::textChanged, this, [this](const QString &text) {
        if (m_model) m_model->setSearch(text);
        m_list->verticalScrollBar()->setValue(0);
    });
    layout->addWidget(m_search);
    m_searchRow->hide();
    return m_searchRow;
}

void ChatPanel::setSearchShown(bool shown) {
    m_searchRow->setVisible(shown);
    if (shown) {
        m_search->setFocus(Qt::ShortcutFocusReason);
        m_search->selectAll();
    } else {
        m_search->clear();
    }
}

QFrame *ChatPanel::buildPinBar() {
    m_pinBar = new QFrame;
    m_pinBar->setObjectName(QStringLiteral("pinBar"));
    m_pinBar->setFixedHeight(0);
    m_pinBar->setCursor(Qt::PointingHandCursor);
    m_pinBar->installEventFilter(this);
    auto *layout = new QHBoxLayout(m_pinBar);
    layout->setContentsMargins(16, 0, 8, 0);
    layout->setSpacing(8);
    auto *pin = new QLabel;
    pin->setFixedSize(14, 14);
    pin->setPixmap(icons::pixmap(QStringLiteral("g-pin"), color(ink3), 14, 2));
    layout->addWidget(pin);
    m_pinAuthor = makeLabel(control, ink3);
    layout->addWidget(m_pinAuthor);
    m_pinSummary = new ElidedLabel;
    m_pinSummary->setFont(tokens::font(controlStrong));
    {
        QPalette palette = m_pinSummary->palette();
        palette.setColor(QPalette::WindowText, color(ink));
        m_pinSummary->setPalette(palette);
    }
    m_pinSummary->setFixedHeight(controlStrong.line);
    m_pinSummary->setObjectName(QStringLiteral("pinSummary"));
    m_pinSummary->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Fixed);
    layout->addWidget(m_pinSummary, 1);
    auto *nav = new QHBoxLayout;
    nav->setSpacing(2);
    m_pinPrev = new ui::IconButton(QStringLiteral("g-chevl"), ui::IconButton::Size::S20);
    m_pinPrev->setTip(QStringLiteral("Previous proposal"));
    m_pinCount = makeLabel(monoSmall, ink3);
    m_pinNext = new ui::IconButton(QStringLiteral("g-chevr"), ui::IconButton::Size::S20);
    m_pinNext->setTip(QStringLiteral("Next proposal"));
    nav->addWidget(m_pinPrev);
    nav->addWidget(m_pinCount);
    nav->addWidget(m_pinNext);
    layout->addLayout(nav);
    const auto step = [this](int delta) {
        const int n = m_model ? int(m_model->openProposals().size()) : 0;
        if (n <= 0) return;
        m_pinIndex = (m_pinIndex + delta + n) % n;
        updatePinBar();
    };
    connect(m_pinPrev, &QAbstractButton::clicked, this, [step] { step(-1); });
    connect(m_pinNext, &QAbstractButton::clicked, this, [step] { step(1); });
    m_pinGrow = new QVariantAnimation(this);
    m_pinGrow->setEasingCurve(easing());
    connect(m_pinGrow, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
        m_pinBar->setFixedHeight(value.toInt());
    });
    return m_pinBar;
}

// ---- inputs ----------------------------------------------------------------

void ChatPanel::setReview(ReviewModel *model) {
    if (m_model == model) return;
    if (m_model) disconnect(m_model, nullptr, this, nullptr);
    m_model = model;
    if (m_model) {
        m_model->setWelcomeShown(true);
        connect(m_model, &ReviewModel::changed, this, &ChatPanel::scheduleRebuild);
        connect(m_model, &ReviewModel::countsChanged, this, &ChatPanel::updateCounts);
        connect(m_model, &ReviewModel::currentChanged, this, &ChatPanel::updateCurrent);
    }
    rebuild();
}

void ChatPanel::setSnapshot(const QJsonObject &snapshot) {
    const QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
    const QJsonObject timelines = snapshot.value(QStringLiteral("proposalTimelines")).toObject();
    if (timeline == m_timeline && timelines == m_proposalTimelines) return;
    m_timeline = timeline;
    m_proposalTimelines = timelines;
    // A colour card whose frames never came (a newer preview request stops an
    // older one) asks again when the rebuild finds it open.
    for (auto it = m_colourRequested.begin(); it != m_colourRequested.end();) {
        if (m_effectFrames.contains(QStringLiteral("after:") + *it)) ++it;
        else it = m_colourRequested.erase(it);
    }
    scheduleRebuild();
}

void ChatPanel::setNamedTimeline(const QJsonObject &timeline) {
    if (timeline == m_named) return;
    m_named = timeline;
    scheduleRebuild();
}

void ChatPanel::setFps(double fps) {
    if (!(fps > 0) || fps == m_fps) return;
    m_fps = fps;
    m_composer->setTimecode(review::timecode(m_playhead, m_fps));
    scheduleRebuild();
}

void ChatPanel::setPlayhead(double seconds) {
    m_playhead = seconds;
    m_composer->setTimecode(review::timecode(seconds, m_fps));
}

void ChatPanel::setConnected(bool connected) {
    if (m_connected == connected) return;
    m_connected = connected;
    scheduleRebuild();
}

QString ChatPanel::reviewLink(double seconds) const { return m_reviewLink ? m_reviewLink(seconds) : QString(); }

void ChatPanel::setModels(const QStringList &models, const QString &current, bool choosable) {
    m_models = models;
    m_currentModel = current;
    m_modelsChoosable = choosable;
}

void ChatPanel::setNextSteps(const QJsonArray &steps) {
    m_steps = steps;
    refillWelcome();
    updateSuggestions();
}

void ChatPanel::setCheckRows(const QJsonArray &rows) {
    m_checkRows = rows;
    updateSuggestions();
}

void ChatPanel::setPreviewProposal(const QString &proposalId) {
    if (m_previewProposal == proposalId) return;
    m_previewProposal = proposalId;
    for (ProposalCard *card : cards()) {
        const ReviewModel::Proposal *proposal = m_model ? m_model->proposal(card->proposalId()) : nullptr;
        card->setPreview(!proposalId.isEmpty() && proposal && proposal->hasVersion(proposalId));
    }
}

bool ChatPanel::setEffectPreview(const QString &key, const QImage &frame) {
    if (!key.startsWith(QLatin1String("before:")) && !key.startsWith(QLatin1String("after:"))) return false;
    m_effectFrames.insert(key, frame);
    const QString version = key.section(QLatin1Char(':'), 1);
    for (ProposalCard *card : cards())
        if (card->shownId() == version)
            card->setColourFrames(m_effectFrames.value(QStringLiteral("before:") + version),
                                  m_effectFrames.value(QStringLiteral("after:") + version));
    return true;
}

void ChatPanel::markSettledHere(const QString &proposalId, bool accepted) {
    m_settledHere.insert(proposalId, accepted);
    m_settledAt.insert(proposalId, QDateTime::currentDateTime());
    scheduleRebuild();
}

QJsonObject ChatPanel::colourPreviewRequest(const QString &versionId) const {
    if (!m_model) return {};
    const ReviewModel::Proposal *proposal = m_model->proposal(versionId);
    if (!proposal) return {};
    const ColourChange change = colourChange(*proposal, versionId);
    if (change.clip.isEmpty()) return {};
    const QJsonObject clip = findClip(m_timeline, change.clip);
    // At the playhead when it is over the clip, else the clip's middle.
    const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
    const double end = start + clip.value(QStringLiteral("duration")).toDouble();
    const double at = m_playhead >= start && m_playhead < end ? m_playhead : (start + end) / 2.0;
    QJsonObject before{{QStringLiteral("fx"), change.fx}, {QStringLiteral("key"), QStringLiteral("before:") + versionId}};
    // A new effect's "before" is the effect at its defaults: the clip as it is.
    if (change.hadBefore) before.insert(QStringLiteral("params"), change.before);
    const QJsonObject after{{QStringLiteral("fx"), change.fx},
                            {QStringLiteral("params"), change.after},
                            {QStringLiteral("key"), QStringLiteral("after:") + versionId}};
    return QJsonObject{{QStringLiteral("type"), QStringLiteral("effect_previews")},
                       {QStringLiteral("clip"), change.clip},
                       {QStringLiteral("at"), at},
                       {QStringLiteral("effects"), QJsonArray{before, after}},
                       {QStringLiteral("width"), 160},
                       {QStringLiteral("key"), QStringLiteral("proposal:") + versionId}};
}

ChatPanel::ColourChange ChatPanel::colourChange(const ReviewModel::Proposal &proposal, const QString &versionId) const {
    ColourChange out;
    const QList<ReviewModel::ClipChange> changes = versionChanges(proposal, versionId);
    if (changes.size() != 1) return out;
    const ReviewModel::ClipChange &change = changes.constFirst();
    if (!change.effects || change.transition || change.added || change.removed) return out;
    const QJsonObject was = findClip(m_timeline, change.clip);
    const QJsonObject now = findClip(m_proposalTimelines.value(versionId).toObject(), change.clip);
    if (now.isEmpty()) return out;
    const QJsonArray before = was.value(QStringLiteral("effects")).toArray();
    const QJsonArray after = now.value(QStringLiteral("effects")).toArray();
    for (qsizetype index = 0; index < after.size(); ++index) {
        const QJsonObject effect = after.at(index).toObject();
        const QString fx = effect.value(QStringLiteral("fx")).toString();
        QJsonObject old;
        bool had = false;
        for (qsizetype other = 0; other < before.size(); ++other) {
            const QJsonObject candidate = before.at(other).toObject();
            if (candidate.value(QStringLiteral("fx")).toString() == fx) {
                old = candidate;
                had = true;
                break;
            }
        }
        const QJsonObject oldParams = staticParams(old.value(QStringLiteral("params")).toObject());
        const QJsonObject newParams = staticParams(effect.value(QStringLiteral("params")).toObject());
        if (had && oldParams == newParams) continue;
        if (!isColourEffect(fx)) return out;
        out.clip = change.clip;
        out.fx = fx;
        out.hadBefore = had;
        out.before = oldParams;
        out.after = newParams;
        out.title = QStringLiteral("%1 · %2").arg(change.name.isEmpty() ? clipName(change.clip) : change.name, effectLabel(fx));
        QStringList parts;
        for (auto it = newParams.begin(); it != newParams.end(); ++it) {
            if (had && oldParams.value(it.key()) == it.value()) continue;
            const QString value = paramValue(it.value());
            if (!value.isEmpty()) parts << QStringLiteral("%1 %2").arg(effectLabel(it.key()), value);
        }
        out.params = parts.join(QStringLiteral(", "));
        return out;
    }
    return out;
}

// ---- building the list --------------------------------------------------------

void ChatPanel::scheduleRebuild() {
    if (m_rebuildQueued) return;
    m_rebuildQueued = true;
    QTimer::singleShot(0, this, [this] {
        if (m_rebuildQueued) rebuild();
    });
}

QString ChatPanel::trackName(const QString &trackId) const {
    for (const QJsonObject *timeline : {&m_named, &m_timeline})
        for (const QJsonValue &value : timeline->value(QStringLiteral("tracks")).toArray()) {
            const QJsonObject track = value.toObject();
            if (track.value(QStringLiteral("id")).toString() != trackId) continue;
            const QString name = track.value(QStringLiteral("name")).toString();
            if (!name.isEmpty()) return name;
        }
    return trackId;
}

QString ChatPanel::clipName(const QString &clipId) const {
    for (const QJsonObject *timeline : {&m_named, &m_timeline}) {
        const QString name = findClip(*timeline, clipId).value(QStringLiteral("name")).toString();
        if (!name.isEmpty()) return name;
    }
    return clipId;
}

QList<ReviewModel::ClipChange> ChatPanel::versionChanges(const ReviewModel::Proposal &proposal, const QString &versionId) const {
    if (versionId.isEmpty() || versionId == proposal.id) {
        if (!proposal.changes.isEmpty() || !m_proposalTimelines.contains(proposal.id)) return proposal.changes;
    }
    const QString id = versionId.isEmpty() ? proposal.id : versionId;
    if (!m_proposalTimelines.contains(id)) return {};
    return ReviewModel::diffTimelines(m_timeline, m_proposalTimelines.value(id).toObject());
}

bool ChatPanel::repliesOpenByDefault(const ReviewModel::Thread &thread) const {
    // Open while something waits on them: a pending proposal, a failure,
    // one's own reply; folded once settled or when they are only notes.
    if (thread.resolved()) return false;
    if (!thread.openProposal.isEmpty()) return true;
    for (const ReviewModel::Comment &reply : thread.replies)
        if (reply.error || (m_model && m_model->isOwn(reply))) return true;
    return false;
}

void ChatPanel::connectRow(CommentRow *row) {
    connect(row, &CommentRow::seekRequested, this, &ChatPanel::seekRequested);
    connect(row, &CommentRow::hoverChanged, this, [this, row](bool hovered) {
        if (row->threadId().isEmpty()) return;
        emit threadHovered(hovered ? row->threadId() : QString());
    });
    connect(row, &CommentRow::pressed, this, [this, row] {
        m_selectedComment = row->commentId();
        for (CommentRow *other : rows()) other->setSelected(other == row && !row->commentId().isEmpty());
        if (!row->commentId().isEmpty()) emit commentSelected(row->commentId());
        if (m_model && !row->isReply() && !row->threadId().isEmpty() && m_model->thread(row->threadId())) {
            m_model->setCurrentThread(row->threadId());
            emit currentThreadChosen(row->threadId());
        }
    });
    connect(row, &CommentRow::replyClicked, this, [this, row] { setReplyTarget(row->threadId()); });
    connect(row, &CommentRow::ringClicked, this, [this, row] {
        const ReviewModel::Thread *thread = m_model ? m_model->thread(row->threadId()) : nullptr;
        if (thread) emit resolveRequested(thread->id, !thread->resolved());
    });
    connect(row, &CommentRow::moreClicked, this, [this, row] { showRowMenu(row); });
    connect(row, &CommentRow::tagClicked, this, [this](const QString &tag) {
        if (!m_model) return;
        m_model->toggleTag(tag);
        m_list->verticalScrollBar()->setValue(0);
    });
    connect(row, &CommentRow::foldToggled, this, [this, row](bool open) {
        m_foldState.insert(row->threadId(), open);
        if (ThreadWidget *thread = threadWidget(row->threadId())) thread->setRepliesShown(open);
        QTimer::singleShot(0, this, &ChatPanel::updatePinBar);
    });
    connect(row, &CommentRow::stepsToggled, this, [this, row](bool open) {
        if (open) m_stepsOpen.insert(row->commentId());
        else m_stepsOpen.remove(row->commentId());
    });
    connect(row, &CommentRow::suggestionChosen, this, [this](const QString &request) { m_composer->fill(request); });
    connect(row, &CommentRow::stopClicked, this, &ChatPanel::stopRequested);
    connect(row, &CommentRow::askAgainClicked, this, [this, row] {
        for (const LocalTurn &turn : std::as_const(m_turns))
            if (turn.kerf == row || turn.rows.contains(row)) {
                emit askAgainInThread(turn.text, turn.threadId);
                return;
            }
    });
}

CommentRow *ChatPanel::buildRow(const ReviewModel::Comment &comment, const ReviewModel::Thread &thread, bool root) {
    auto *row = new CommentRow(CommentRow::Kind::Record, QString(), comment.created);
    row->setCommentId(comment.id);
    row->setThreadId(thread.id);
    row->setFps(m_fps);
    const bool own = m_model->isOwn(comment);
    const QString kind = comment.author.kind.isEmpty() ? QStringLiteral("person") : comment.author.kind;
    row->setAuthor(kind, own ? QString() : ReviewModel::initials(comment.author));
    row->setName(m_model->displayName(comment.author));
    QString role = comment.author.role;
    if (role.isEmpty() && kind == QLatin1String("kerf")) role = QStringLiteral("agent");
    // A person's role is said once, on the thread's note.
    if (!root && comment.author.isPerson()) role.clear();
    row->setRole(role);
    if (root && comment.hasAt()) row->setMark(comment.at, comment.duration, m_fps);
    row->setTime(comment.created, comment.revision);
    row->setError(comment.error);
    row->setDrawing(comment.hasDrawing());
    row->setText(comment.body);
    const bool record = thread.origin == ReviewModel::Origin::Record;
    if (root) {
        row->setMuted(thread.resolved());
        if (record) row->setRing(true, thread.resolved());
        if (!comment.qcKey.isEmpty() && thread.open) row->addTag(QStringLiteral("Failed"), TagButton::Style::Failed);
        if (thread.flagged) row->addTag(QStringLiteral("Blocking"), TagButton::Style::Blocking, QString::fromLatin1(ReviewModel::kBlockingTag));
        for (const QString &tag : thread.tags) row->addTag(QStringLiteral("#") + tag, TagButton::Style::Tag, tag);
        if (thread.acceptedRevision >= 0)
            row->addTag(QStringLiteral("Accepted in rev %1").arg(thread.acceptedRevision), TagButton::Style::Accepted);
    }
    if (comment.error) {
        ChatExpander *again = row->addLink(QStringLiteral("Ask again"));
        ChatExpander *copy = row->addLink(QStringLiteral("Copy"));
        // The request that failed: the latest of one's own notes before it.
        QString request = thread.root.body;
        for (const ReviewModel::Comment &reply : thread.replies) {
            if (reply.id == comment.id) break;
            if (m_model->isOwn(reply)) request = reply.body;
        }
        const QString threadId = thread.id;
        connect(again, &QAbstractButton::clicked, this, [this, request, threadId] {
            emit askAgainInThread(request, threadId);
        });
        connect(copy, &QAbstractButton::clicked, this, [body = comment.body] { QApplication::clipboard()->setText(body); });
    }
    if (!comment.steps.isEmpty()) {
        row->setSteps(comment.steps);
        row->setRepliesExpanded(m_stepsOpen.contains(comment.id));
    }
    row->setActions(record, root && record && !own, true);
    row->setSelected(!m_selectedComment.isEmpty() && comment.id == m_selectedComment);
    connectRow(row);
    return row;
}

ProposalCard *ChatPanel::buildCard(const ReviewModel::Proposal &proposal, const ReviewModel::Thread &thread) {
    // A settled proposal keeps a card only when it was settled here.
    QString settledKey;
    for (auto it = m_settledHere.cbegin(); it != m_settledHere.cend(); ++it)
        if (proposal.hasVersion(it.key()) || it.key() == proposal.id) settledKey = it.key();
    if (!proposal.isOpen() && settledKey.isEmpty()) return nullptr;
    auto *card = new ProposalCard(proposal.id);
    const QList<ReviewModel::Proposal> open = m_model->openProposals();
    const bool pinned = proposal.isOpen() && !open.isEmpty() && open.constFirst().id == proposal.id;
    CardState &state = m_cardState[proposal.id];
    if (!state.openSet) {
        state.open = pinned;
        state.openSet = true;
    }
    QString shown = state.shown;
    if (!shown.isEmpty() && !proposal.hasVersion(shown)) shown.clear();
    const QString versionId = shown.isEmpty() ? proposal.id : shown;
    card->setPinned(pinned, proposal.blocking);
    card->setTitle(proposal.summary);

    // The changes: from the timelines when there are some, else the script's words.
    QList<ProposalCard::Change> rows;
    for (const ReviewModel::ClipChange &change : versionChanges(proposal, versionId)) {
        ProposalCard::Change row;
        const QString name = change.transition ? QStringLiteral("Transition") : (change.name.isEmpty() ? clipName(change.clip) : change.name);
        row.name = change.track.isEmpty() ? name : QStringLiteral("%1 · %2").arg(name, trackName(change.track));
        if (change.removed) {
            row.target = QStringLiteral("removed");
        } else if (change.added) {
            row.target = QStringLiteral("→ %1").arg(review::timecode(change.newStart, m_fps));
            row.shift = QStringLiteral("new");
        } else if (change.moved) {
            row.target = QStringLiteral("→ %1").arg(review::timecode(change.newStart, m_fps));
            row.shift = review::signedFrames(std::llround(change.shift() * m_fps));
        } else if (change.trimmed) {
            row.target = QStringLiteral("→ %1").arg(review::timecode(change.newEnd, m_fps));
            row.shift = review::signedFrames(
                std::llround(((change.newEnd - change.newStart) - (change.oldEnd - change.oldStart)) * m_fps));
        } else {
            QStringList what;
            if (change.effects) what << QStringLiteral("effects");
            if (change.properties) what << QStringLiteral("properties");
            if (change.toggled) what << QStringLiteral("enabled");
            if (change.retimed) what << QStringLiteral("speed");
            if (change.relinked) what << QStringLiteral("link");
            if (change.retracked) what << QStringLiteral("track");
            row.target = what.join(QStringLiteral(", "));
        }
        rows << row;
    }
    if (rows.isEmpty() && m_describer) {
        QString script = proposal.script;
        for (const ReviewModel::Version &version : proposal.versions)
            if (version.proposalId == versionId) script = version.script;
        for (const QString &line : script.split(QLatin1Char('\n'))) {
            const QString text = line.trimmed();
            if (text.isEmpty() || text.startsWith(QLatin1Char('#')) || text.startsWith(QLatin1String("use "))) continue;
            const ProposalChange change = m_describer(text);
            rows << ProposalCard::Change{change.description, change.detail, QString()};
        }
    }
    card->setChanges(rows);

    // Versions, each with where its first move lands.
    QList<ProposalCard::VersionItem> versions;
    for (const ReviewModel::Version &version : proposal.versions) {
        ProposalCard::VersionItem item{version.version, version.proposalId, QStringLiteral("v%1").arg(version.version)};
        for (const ReviewModel::ClipChange &change : versionChanges(proposal, version.proposalId))
            if (change.moved) {
                item.label = QStringLiteral("v%1 %2 · %3 fr")
                                 .arg(version.version)
                                 .arg(review::timecode(change.newStart, m_fps))
                                 .arg(review::signedFrames(std::llround(change.shift() * m_fps)).chopped(1));
                break;
            }
        versions << item;
    }
    card->setVersions(versions, versionId);

    // The colour chip, for a colour change on one clip.
    const ColourChange colour = colourChange(proposal, versionId);
    if (!colour.clip.isEmpty()) {
        card->setColour(ProposalCard::Colour{colour.clip, colour.fx, colour.title, colour.params});
        card->setColourFrames(m_effectFrames.value(QStringLiteral("before:") + versionId),
                              m_effectFrames.value(QStringLiteral("after:") + versionId));
    }

    // The diff: the document's changed lines, else the script.
    QString diff = proposal.documentDiff;
    QString script = proposal.script;
    for (const ReviewModel::Version &version : proposal.versions)
        if (version.proposalId == versionId) {
            diff = version.documentDiff;
            script = version.script;
        }
    QStringList lines;
    for (const QString &line : diff.split(QLatin1Char('\n'))) {
        if (line.startsWith(QLatin1String("+++")) || line.startsWith(QLatin1String("---"))) continue;
        if (line.startsWith(QLatin1Char('+')) || line.startsWith(QLatin1Char('-'))) lines << line;
    }
    if (lines.isEmpty())
        for (const QString &line : script.split(QLatin1Char('\n')))
            if (!line.trimmed().isEmpty()) lines << line;
    card->setDiff(lines);

    card->setOpen(state.open);
    card->setScriptShown(state.script);
    card->setPreview(!m_previewProposal.isEmpty() && proposal.hasVersion(m_previewProposal));
    card->setPrimary(pinned || proposal.blocking);
    const ReviewModel::Comment &root = thread.root;
    if (root.author.isPerson() && !m_model->isOwn(root) && thread.origin == ReviewModel::Origin::Record)
        card->setAcceptTip(QStringLiteral("Accept · resolves %1's note").arg(firstName(root.author.name)));
    card->setGuest(m_model->guestMode());
    if (proposal.stale) card->setAcceptBlocked(QStringLiteral("Out of date · ask for a new version"));
    if (!m_connected) card->setActionsEnabled(false);
    if (!proposal.isOpen()) {
        const bool accepted = proposal.status == QLatin1String("accepted");
        const QString when = review::age(m_settledAt.value(settledKey), QDateTime::currentDateTime());
        card->setSettled(accepted ? (proposal.appliedRevision >= 0
                                         ? QStringLiteral("Accepted in rev %1 · You · %2").arg(proposal.appliedRevision).arg(when)
                                         : QStringLiteral("Accepted · You · %1").arg(when))
                                  : QStringLiteral("Rejected · You · %1").arg(when),
                         accepted);
    }

    const QString id = proposal.id;
    connect(card, &ProposalCard::openToggled, this, [this, id, card](bool openNow) {
        m_cardState[id].open = openNow;
        if (openNow && card->hasColour() && !m_colourRequested.contains(card->shownId())) {
            m_colourRequested.insert(card->shownId());
            emit colourPreviewWanted(card->shownId());
        }
        QTimer::singleShot(0, this, &ChatPanel::updatePinBar);
    });
    connect(card, &ProposalCard::scriptToggled, this, [this, id](bool shownNow) { m_cardState[id].script = shownNow; });
    connect(card, &ProposalCard::previewToggled, this, [this, card](bool on) { emit previewRequested(card->shownId(), on); });
    connect(card, &ProposalCard::versionChosen, this, [this, id](const QString &version) {
        m_cardState[id].shown = version == id ? QString() : version;
        // Choosing an older version previews it.
        emit previewRequested(version, true);
        emit shownVersionChanged(id, version);
        scheduleRebuild();
    });
    connect(card, &ProposalCard::acceptRequested, this, &ChatPanel::acceptRequested);
    connect(card, &ProposalCard::rejectRequested, this, &ChatPanel::rejectRequested);
    if (state.open && card->hasColour() && !m_colourRequested.contains(versionId)) {
        m_colourRequested.insert(versionId);
        QTimer::singleShot(0, this, [this, versionId] { emit colourPreviewWanted(versionId); });
    }
    return card;
}

void ChatPanel::stashLocalRows() {
    m_welcomeRow->setParent(m_stash);
    for (LocalTurn &turn : m_turns)
        for (const QPointer<CommentRow> &row : std::as_const(turn.rows))
            if (row) row->setParent(m_stash);
    for (const QPointer<CommentRow> &row : std::as_const(m_pendingNotes))
        if (row) row->setParent(m_stash);
}

void ChatPanel::rebuild() {
    m_rebuildQueued = false;
    const int scrolled = m_list->verticalScrollBar()->value();
    stashLocalRows();
    // Later, not now: a row of an old thread may be the sender of the
    // signal that asked for this rebuild.
    for (ThreadWidget *thread : std::as_const(m_threads)) {
        m_listLayout->removeWidget(thread);
        thread->hide();
        thread->setParent(nullptr);
        thread->deleteLater();
    }
    m_threads.clear();

    bool first = true;
    const auto addThread = [this, &first](ThreadWidget *thread) {
        m_listLayout->insertWidget(int(m_threads.size()), thread);
        m_threads.append(thread);
        // Shown now, not by the layout's queued show, so the list measures it.
        thread->show();
        first = false;
    };
    const QList<ReviewModel::Thread> threads = m_model ? m_model->threads() : QList<ReviewModel::Thread>{};
    const bool welcome = !m_model || std::any_of(threads.begin(), threads.end(), [](const ReviewModel::Thread &thread) {
        return thread.origin == ReviewModel::Origin::Welcome;
    });
    if (welcome) {
        auto *thread = new ThreadWidget(QString::fromLatin1(ReviewModel::kWelcomeId), first);
        thread->setRoot(m_welcomeRow);
        m_welcomeRow->setThreadId(thread->threadId());
        m_welcomeRow->show();
        addThread(thread);
    }
    for (const ReviewModel::Thread &thread : threads) {
        if (thread.origin == ReviewModel::Origin::Welcome) continue;
        auto *widget = new ThreadWidget(thread.id, first);
        CommentRow *root = buildRow(thread.root, thread, true);
        widget->setRoot(root);
        root->setCurrent(m_model->isCurrent(thread.id));
        QList<CommentRow *> replies;
        for (const ReviewModel::Comment &reply : thread.replies) {
            CommentRow *row = buildRow(reply, thread, false);
            widget->addReply(row);
            replies << row;
        }
        // Each proposal's card goes under the first comment that carries it.
        QSet<QString> carded;
        const auto attach = [&](CommentRow *row, const QString &carried, bool isRoot) {
            if (carried.isEmpty()) return;
            const ReviewModel::Proposal *proposal = m_model->proposal(carried);
            if (!proposal || carded.contains(proposal->id)) return;
            carded.insert(proposal->id);
            ProposalCard *card = buildCard(*proposal, thread);
            if (!card) return;
            if (isRoot || card->isSettled()) row->addBodyWidget(card);
            else row->addWideWidget(card);
        };
        attach(root, thread.origin == ReviewModel::Origin::Proposal ? thread.openProposal : thread.root.proposal, true);
        for (qsizetype index = 0; index < thread.replies.size(); ++index)
            attach(replies.at(index), thread.replies.at(index).proposal, false);
        const bool open = m_foldState.value(thread.id, repliesOpenByDefault(thread));
        root->setReplyFold(int(thread.replies.size()), open);
        widget->setRepliesShown(open);
        addThread(widget);
    }
    placeLocalRows(first);
    updateCounts();
    updateHeader();
    // The people and notes the composer offers follow the review.
    updateSuggestions();
    syncContent();
    m_list->verticalScrollBar()->setValue(scrolled);
    QTimer::singleShot(0, this, [this] {
        m_list->updateFades();
        updatePinBar();
        if (!m_placed && isVisible()) placeFirst();
    });
}

void ChatPanel::placeLocalRows(bool &first) {
    // Notes posted with comment_add, then the agent turns, each in its
    // thread when it has one in the list, else as a thread of its own.
    const auto place = [this, &first](const QString &threadId, const QList<CommentRow *> &rows) {
        if (rows.isEmpty()) return;
        if (ThreadWidget *thread = threadWidget(threadId)) {
            for (CommentRow *row : rows) {
                thread->addReply(row);
                row->setThreadId(threadId);
                row->show();
            }
            thread->setRepliesShown(true);
            if (CommentRow *root = thread->root())
                if (ChatExpander *fold = root->replyFold()) fold->setOpenQuietly(true);
            return;
        }
        auto *thread = new ThreadWidget(QStringLiteral("local:%1").arg(m_threads.size()), first);
        thread->setRoot(rows.constFirst());
        rows.constFirst()->show();
        for (qsizetype index = 1; index < rows.size(); ++index) {
            thread->addReply(rows.at(index));
            rows.at(index)->show();
        }
        thread->setRepliesShown(true);
        m_listLayout->insertWidget(int(m_threads.size()), thread);
        m_threads.append(thread);
        thread->show();
        first = false;
    };
    for (auto it = m_pendingNotes.cbegin(); it != m_pendingNotes.cend(); ++it)
        if (it.value()) place(m_pendingThreads.value(it.key()), {it.value().data()});
    for (const LocalTurn &turn : std::as_const(m_turns)) {
        QList<CommentRow *> rows;
        for (const QPointer<CommentRow> &row : turn.rows)
            if (row) rows << row.data();
        place(turn.threadId, rows);
    }
}

void ChatPanel::refillWelcome() {
    QList<QPair<QString, QString>> suggestions;
    for (const QJsonValue &value : std::as_const(m_steps)) {
        const QJsonObject step = value.toObject();
        const QString label = value.isString() ? value.toString() : step.value(QStringLiteral("label")).toString();
        const QString request = value.isString() ? value.toString() : step.value(QStringLiteral("request")).toString();
        if (!label.isEmpty()) suggestions.append({label, request.isEmpty() ? label : request});
    }
    m_welcomeRow->setSuggestions(suggestions);
}

void ChatPanel::updateCounts() {
}

void ChatPanel::updateHeader() {
    // A filter that hides rows shows on the ⋯ button.
    const bool chip = m_model && m_model->chip() != ReviewModel::Chip::All;
    const bool from = m_model && m_model->from() != ReviewModel::From::Anyone;
    m_filterDot->setVisible(chip || from);
}

bool ChatPanel::filterDotShown() const { return !m_filterDot->isHidden(); }

void ChatPanel::updateCurrent() {
    for (ThreadWidget *thread : std::as_const(m_threads))
        if (CommentRow *root = thread->root()) root->setCurrent(m_model && m_model->isCurrent(thread->threadId()));
}

// ---- threads -------------------------------------------------------------------

QList<ThreadWidget *> ChatPanel::threadWidgets() const { return m_threads; }

ThreadWidget *ChatPanel::threadWidget(const QString &threadId) const {
    if (threadId.isEmpty()) return nullptr;
    for (ThreadWidget *thread : m_threads)
        if (thread->threadId() == threadId) return thread;
    return nullptr;
}

CommentRow *ChatPanel::rowFor(const QString &commentId) const {
    if (commentId.isEmpty()) return nullptr;
    for (CommentRow *row : rows())
        if (row->commentId() == commentId) return row;
    return nullptr;
}

ProposalCard *ChatPanel::card(const QString &proposalId) const {
    const ReviewModel::Proposal *proposal = m_model ? m_model->proposal(proposalId) : nullptr;
    for (ProposalCard *card : cards())
        if (card->proposalId() == proposalId || (proposal && card->proposalId() == proposal->id)) return card;
    return nullptr;
}

QList<ProposalCard *> ChatPanel::cards() const { return m_content->findChildren<ProposalCard *>(); }

void ChatPanel::syncContent() {
    // The scroll area sizes its content on a posted layout request; size it
    // now, so the scroll range is the content's before scrolling.
    QCoreApplication::sendPostedEvents(m_content, QEvent::LayoutRequest);
    m_content->layout()->activate();
    const int width = m_list->viewport()->width();
    const int height = std::max(m_list->viewport()->height(),
                                m_content->hasHeightForWidth() ? m_content->heightForWidth(width) : m_content->sizeHint().height());
    if (m_content->size() != QSize(width, height)) m_content->resize(width, height);
}

void ChatPanel::scrollToThread(const QString &threadId) {
    ThreadWidget *thread = threadWidget(threadId);
    if (!thread || !thread->root()) return;
    syncContent();
    // Its highlight starts 12 under the list's top.
    const int top = thread->root()->mapTo(m_content, QPoint()).y();
    m_list->verticalScrollBar()->setValue(std::max(0, top - kListTop));
}

void ChatPanel::openThread(const QString &threadId, bool flash) {
    if (!m_model) return;
    const ReviewModel::Thread *thread = m_model->threadOf(threadId);
    if (!thread) return;
    const QString id = thread->id;
    // A thread the filters hide is shown again.
    if (!m_model->passes(*thread)) m_model->clearFilters();
    m_model->setCurrentThread(id);
    m_foldState.insert(id, true);
    if (m_rebuildQueued) rebuild();
    if (ThreadWidget *widget = threadWidget(id)) {
        widget->setRepliesShown(true);
        if (CommentRow *root = widget->root())
            if (ChatExpander *fold = root->replyFold()) fold->setOpenQuietly(true);
    }
    m_placed = true;
    QTimer::singleShot(0, this, [this, id, flash] {
        scrollToThread(id);
        if (flash)
            if (ThreadWidget *widget = threadWidget(id))
                if (CommentRow *root = widget->root()) root->flash();
    });
}

void ChatPanel::placeFirst() {
    // After the rebuild that lists the threads (it calls this again).
    if (m_placed || !m_model || m_rebuildQueued) return;
    QString current = m_model->currentThread();
    if (current.isEmpty()) current = m_model->threadAt(m_playhead, 0.5);
    if (current.isEmpty()) {
        const QList<ReviewModel::Thread> threads = m_model->threads();
        for (const ReviewModel::Thread &thread : threads)
            if (thread.origin == ReviewModel::Origin::Record && thread.open) {
                current = thread.id;
                break;
            }
    }
    if (current.isEmpty() || !threadWidget(current)) return;
    m_placed = true;
    m_model->setCurrentThread(current);
    scrollToThread(current);
    if (!m_reviewWorkspace) return;
    // In Review, the pinned proposal's Accept row clears the bottom fade by 30.
    const QList<ReviewModel::Proposal> open = m_model->openProposals();
    if (open.isEmpty()) return;
    ProposalCard *pinned = card(open.constFirst().id);
    if (!pinned || !pinned->footer()->isVisible()) return;
    QScrollBar *bar = m_list->verticalScrollBar();
    const int footerBottom = pinned->footer()->mapTo(m_content, QPoint(0, pinned->footer()->height())).y();
    const int wanted = footerBottom - (m_list->viewport()->height() - 30);
    if (wanted > bar->value()) bar->setValue(std::min(wanted, bar->maximum()));
}

// ---- the pin bar -----------------------------------------------------------------

QString ChatPanel::pinBarProposal() const {
    if (!m_model) return {};
    const QList<ReviewModel::Proposal> open = m_model->openProposals();
    if (open.isEmpty()) return {};
    return open.at(m_pinIndex % open.size()).id;
}

void ChatPanel::updatePinBar() {
    const QList<ReviewModel::Proposal> open = m_model ? m_model->openProposals() : QList<ReviewModel::Proposal>{};
    if (open.isEmpty()) {
        m_pinIndex = 0;
        animatePinBar(false);
        return;
    }
    m_pinIndex %= int(open.size());
    const ReviewModel::Proposal &proposal = open.at(m_pinIndex);
    m_pinAuthor->setText(m_model->displayName(proposal.author));
    m_pinAuthor->setFixedWidth(ui::snapWidth(control, m_pinAuthor->text()));
    m_pinSummary->setText(proposal.summary);
    m_pinSummary->setToolTip(QString());
    ui::setTip(m_pinSummary, proposal.summary);
    m_pinCount->setText(QStringLiteral("%1/%2").arg(m_pinIndex + 1).arg(open.size()));
    m_pinCount->setFixedWidth(ui::snapWidth(monoSmall, m_pinCount->text()));
    // Shown while that card's footer is not fully in view.
    bool visible = false;
    if (ProposalCard *card = this->card(proposal.id)) {
        QWidget *footer = card->footer();
        if (footer->isVisible()) {
            const QRect area = m_list->viewport()->rect();
            const QPoint top = footer->mapTo(m_list->viewport(), QPoint(0, 0));
            const int bottom = top.y() + footer->height();
            visible = top.y() >= area.top() - 1 && bottom <= area.bottom() + 1 - kPinMargin;
        }
    }
    animatePinBar(!visible);
}

void ChatPanel::animatePinBar(bool shown) {
    if (shown == m_pinWanted && (m_pinGrow->state() == QAbstractAnimation::Running || m_pinBar->height() == (shown ? kPinHeight : 0)))
        return;
    m_pinWanted = shown;
    m_pinGrow->stop();
    const int target = shown ? kPinHeight : 0;
    const int ms = ui::motionDuration(motion::fade);
    if (ms <= 0 || !isVisible()) {
        m_pinBar->setFixedHeight(target);
        return;
    }
    m_pinGrow->setDuration(ms);
    m_pinGrow->setStartValue(m_pinBar->height());
    m_pinGrow->setEndValue(target);
    m_pinGrow->start();
}

// ---- the agent turn in progress ---------------------------------------------------------

CommentRow *ChatPanel::beginTurn(const QString &text, const QString &threadId, double at) {
    LocalTurn turn;
    turn.threadId = threadId;
    turn.started = QDateTime::currentDateTimeUtc();
    turn.text = text;
    auto *row = new CommentRow(CommentRow::Kind::User, text, QDateTime::currentDateTime(), m_stash);
    row->setFps(m_fps);
    if (threadId.isEmpty() && std::isfinite(at)) row->setMark(at, 0, m_fps);
    ui::setTip(row->timeLabel(), QStringLiteral("Unsaved · syncs on push"));
    connectRow(row);
    turn.rows << row;
    m_turns << turn;
    scheduleRebuild();
    scheduleFollow();
    return row;
}

CommentRow *ChatPanel::turnKerfRow(bool create) {
    if (m_turns.isEmpty()) {
        if (!create) return nullptr;
        LocalTurn turn;
        turn.started = QDateTime::currentDateTimeUtc();
        m_turns << turn;
    }
    LocalTurn &turn = m_turns.last();
    if (turn.kerf || !create) return turn.kerf;
    auto *row = new CommentRow(CommentRow::Kind::Drafting, QString(), QDateTime(), m_stash);
    row->setFps(m_fps);
    row->setRole(QStringLiteral("agent"));
    connectRow(row);
    turn.kerf = row;
    turn.rows << row;
    scheduleRebuild();
    scheduleFollow();
    return row;
}

void ChatPanel::setTurnBusy(bool busy) {
    if (busy) {
        CommentRow *row = turnKerfRow(true);
        if (row && !row->isDrafting() && !m_turns.last().done) row->setDrafting(true);
    }
    m_composer->setBusy(busy);
}

void ChatPanel::setTurnStatus(const QString &meta) {
    if (m_turns.isEmpty()) return;
    if (CommentRow *row = m_turns.last().kerf) row->setDraftMeta(meta);
}

void ChatPanel::stopTurn() {
    if (m_turns.isEmpty()) return;
    // The stop may come after the turn ended (busy off first): the stopped
    // row stands in for Kerf's answer either way.
    CommentRow *row = turnKerfRow(true);
    if (row) row->setStopped(true);
    m_turns.last().done = true;
}

void ChatPanel::finishTurn() {
    if (m_turns.isEmpty()) return;
    LocalTurn &turn = m_turns.last();
    if (turn.kerf && turn.kerf->isDrafting()) {
        if (turn.kerf->text().isEmpty() && turn.kerf->replies().isEmpty()) {
            turn.rows.removeAll(turn.kerf);
            turn.kerf->deleteLater();
            turn.kerf = nullptr;
            scheduleRebuild();
        } else {
            turn.kerf->finish();
        }
    }
    turn.done = true;
    reconcileLocal();
}

CommentRow *ChatPanel::addPendingNote(const QString &clientId, const QString &text, const QString &threadId, double at) {
    auto *row = new CommentRow(CommentRow::Kind::User, text, QDateTime::currentDateTime(), m_stash);
    row->setCommentId(clientId);
    row->setFps(m_fps);
    if (threadId.isEmpty() && std::isfinite(at)) row->setMark(at, 0, m_fps);
    ui::setTip(row->timeLabel(), QStringLiteral("Saving"));
    connectRow(row);
    m_pendingNotes.insert(clientId, row);
    m_pendingThreads.insert(clientId, threadId);
    scheduleRebuild();
    scheduleFollow();
    return row;
}

void ChatPanel::failPendingNote(const QString &clientId, const QString &message) {
    if (CommentRow *row = m_pendingNotes.value(clientId)) {
        row->setError(true);
        row->errorLabel()->setText(QStringLiteral("Couldn't post"));
        ui::setTip(row->errorLabel(), message);
    }
}

void ChatPanel::reconcileLocal() {
    if (!m_model) return;
    bool changed = false;
    // Notes the log has by their client id.
    for (auto it = m_pendingNotes.begin(); it != m_pendingNotes.end();) {
        if (!it.value() || m_model->comment(it.key())) {
            if (it.value()) it.value()->deleteLater();
            m_pendingThreads.remove(it.key());
            it = m_pendingNotes.erase(it);
            changed = true;
        } else {
            ++it;
        }
    }
    // The agent turn's rows, once kerfd recorded them as comments: the
    // request as one's own comment with its words, Kerf's answer as a
    // reply in that thread written after the turn began.
    QSet<QString> claimed;
    for (LocalTurn &turn : m_turns) {
        const QDateTime since = turn.started.addSecs(-5);
        if (!turn.youRecorded && !turn.rows.isEmpty() && turn.rows.constFirst() && turn.rows.constFirst()->kind() == CommentRow::Kind::User) {
            for (const ReviewModel::Thread &thread : m_model->allThreads()) {
                QList<const ReviewModel::Comment *> comments{&thread.root};
                for (const ReviewModel::Comment &reply : thread.replies) comments << &reply;
                for (const ReviewModel::Comment *comment : comments) {
                    if (claimed.contains(comment->id) || !m_model->isOwn(*comment)) continue;
                    if (comment->body.trimmed() != turn.text.trimmed()) continue;
                    if (comment->created.isValid() && comment->created < since) continue;
                    claimed.insert(comment->id);
                    turn.youRecorded = true;
                    turn.threadId = thread.id;
                    CommentRow *you = turn.rows.takeFirst();
                    if (you) you->deleteLater();
                    changed = true;
                    break;
                }
                if (turn.youRecorded) break;
            }
        }
        if (turn.done && turn.kerf && (turn.youRecorded || turn.threadId.isEmpty())) {
            const ReviewModel::Thread *thread = m_model->thread(turn.threadId);
            bool answered = false;
            if (thread)
                for (const ReviewModel::Comment &reply : thread->replies)
                    if (reply.author.isKerf() && (!reply.created.isValid() || reply.created >= since)) answered = true;
            if (answered) {
                turn.rows.removeAll(turn.kerf);
                turn.kerf->deleteLater();
                turn.kerf = nullptr;
                changed = true;
            }
        }
    }
    // Error rows of a finished, recorded turn go with it.
    for (qsizetype index = m_turns.size() - 1; index >= 0; --index) {
        LocalTurn &turn = m_turns[index];
        turn.rows.removeAll(QPointer<CommentRow>());
        if (turn.done && turn.youRecorded && !turn.kerf) {
            for (const QPointer<CommentRow> &row : std::as_const(turn.rows))
                if (row) row->deleteLater();
            turn.rows.clear();
        }
        if (turn.rows.isEmpty() && (turn.done || index < m_turns.size() - 1)) {
            m_turns.removeAt(index);
            changed = true;
        }
    }
    if (changed) scheduleRebuild();
}

CommentRow *ChatPanel::addComment(CommentRow::Kind kind, const QString &text) {
    switch (kind) {
    case CommentRow::Kind::User: return beginTurn(text, m_replyTarget);
    case CommentRow::Kind::Kerf:
    case CommentRow::Kind::Record: {
        // Kerf's answer in the turn in progress, or a turn of its own.
        if (m_turns.isEmpty() || m_turns.last().kerf || m_turns.last().done) {
            LocalTurn turn;
            turn.started = QDateTime::currentDateTimeUtc();
            if (!m_turns.isEmpty() && m_turns.last().done) turn.threadId = m_turns.last().threadId;
            m_turns << turn;
        }
        auto *row = new CommentRow(CommentRow::Kind::Kerf, text, QDateTime::currentDateTime(), m_stash);
        row->setFps(m_fps);
        row->setRole(QStringLiteral("agent"));
        connectRow(row);
        m_turns.last().kerf = row;
        m_turns.last().rows << row;
        scheduleRebuild();
        scheduleFollow();
        return row;
    }
    case CommentRow::Kind::Error: {
        if (m_turns.isEmpty()) {
            LocalTurn turn;
            turn.started = QDateTime::currentDateTimeUtc();
            m_turns << turn;
        }
        LocalTurn &turn = m_turns.last();
        CommentRow *row = turn.kerf;
        if (row && row->isDrafting()) {
            row->finish();
            row->setText(text);
            row->setError(true);
        } else {
            row = new CommentRow(CommentRow::Kind::Error, text, QDateTime::currentDateTime(), m_stash);
            row->setFps(m_fps);
            row->setRole(QStringLiteral("agent"));
            connectRow(row);
            turn.rows << row;
        }
        ChatExpander *again = row->addLink(QStringLiteral("Ask again"));
        ChatExpander *copy = row->addLink(QStringLiteral("Copy"));
        const QString request = turn.text;
        const QString threadId = turn.threadId;
        connect(again, &QAbstractButton::clicked, this, [this, request, threadId] {
            if (!request.isEmpty()) emit askAgainInThread(request, threadId);
        });
        connect(copy, &QAbstractButton::clicked, this, [row] { QApplication::clipboard()->setText(row->text()); });
        scheduleRebuild();
        scheduleFollow();
        return row;
    }
    case CommentRow::Kind::Drafting: return turnKerfRow(true);
    }
    return nullptr;
}

QList<CommentRow *> ChatPanel::rows() const {
    QList<CommentRow *> out;
    for (ThreadWidget *thread : m_threads)
        for (CommentRow *row : thread->rows())
            if (row != m_welcomeRow) out << row;
    // Local rows the next rebuild places, in their order.
    for (const QPointer<CommentRow> &row : m_pendingNotes)
        if (row && row->parentWidget() == m_stash) out << row.data();
    for (const LocalTurn &turn : m_turns)
        for (const QPointer<CommentRow> &row : turn.rows)
            if (row && row->parentWidget() == m_stash) out << row.data();
    return out;
}

QString ChatPanel::transcript() const {
    QStringList lines;
    for (CommentRow *row : rows()) {
        lines << row->name();
        if (!row->text().isEmpty()) lines << row->text();
    }
    return lines.join(QLatin1Char('\n'));
}

void ChatPanel::refreshTimes() {
    const QDateTime now = QDateTime::currentDateTime();
    m_welcomeRow->setTime(m_welcomeSince);
    m_welcomeRow->refreshTime(now);
    ui::setTip(m_welcomeRow->timeLabel(), QStringLiteral("Since this window opened"));
    for (CommentRow *row : rows()) row->refreshTime(now);
}

void ChatPanel::scheduleFollow() {
    if (!m_follow) return;
    QTimer::singleShot(0, this, [this] {
        syncContent();
        m_list->verticalScrollBar()->setValue(m_list->verticalScrollBar()->maximum());
        m_follow = true;
    });
}

// ---- the composer -------------------------------------------------------------------

QString ChatPanel::shownVersion(const QString &proposalId) const {
    const QString shown = m_cardState.value(proposalId).shown;
    const ReviewModel::Proposal *proposal = m_model && !shown.isEmpty() ? m_model->proposal(proposalId) : nullptr;
    return proposal && proposal->hasVersion(shown) ? shown : proposalId;
}

void ChatPanel::setReplyTarget(const QString &threadId) {
    m_replyTarget = threadId;
    m_editing.clear();
    const ReviewModel::Thread *thread = m_model && !threadId.isEmpty() ? m_model->thread(threadId) : nullptr;
    // A proposal's own thread is not in the review log: kerfd has no parent to reply to.
    if (thread && thread->origin != ReviewModel::Origin::Record) thread = nullptr;
    if (!thread) {
        m_replyTarget.clear();
        m_replyBar->setTarget({}, {});
        return;
    }
    m_replyBar->setTarget(QStringLiteral("Replying to"), m_model->displayName(thread->root.author));
    m_composer->input()->setFocus(Qt::OtherFocusReason);
}

void ChatPanel::setEditing(const QString &commentId) {
    const ReviewModel::Comment *comment = m_model && !commentId.isEmpty() ? m_model->comment(commentId) : nullptr;
    m_replyTarget.clear();
    m_editing = comment ? commentId : QString();
    if (!comment) {
        m_replyBar->setTarget({}, {});
        return;
    }
    m_replyBar->setTarget(QStringLiteral("Editing"), QStringLiteral("your note"));
    m_composer->fill(comment->body);
}

void ChatPanel::updateComposer(bool busy) { m_composer->setBusy(busy); }

QStringList ChatPanel::mentionCandidates() const {
    QStringList names{QStringLiteral("Kerf")};
    if (!m_model) return names;
    for (const ReviewModel::Thread &thread : m_model->allThreads()) {
        QList<const ReviewModel::Comment *> comments{&thread.root};
        for (const ReviewModel::Comment &reply : thread.replies) comments << &reply;
        for (const ReviewModel::Comment *comment : comments)
            if (comment->author.isAgent() && !comment->author.name.isEmpty() && !names.contains(comment->author.name))
                names << comment->author.name;
    }
    for (const ReviewModel::Person &person : m_model->people())
        if (!person.name.isEmpty() && !names.contains(person.name)) names << person.name;
    return names;
}

void ChatPanel::updateSuggestions() {
    m_suggestionList.clear();
    // Kerf's next steps.
    for (const QJsonValue &value : std::as_const(m_steps)) {
        const QJsonObject step = value.toObject();
        const QString label = value.isString() ? value.toString() : step.value(QStringLiteral("label")).toString();
        const QString request = value.isString() ? value.toString() : step.value(QStringLiteral("request")).toString();
        if (!label.isEmpty()) m_suggestionList << Suggestion{label, request.isEmpty() ? label : request, false};
    }
    if (m_model) {
        // Each person's open notes.
        QHash<QString, QStringList> notes;
        QStringList order;
        for (const ReviewModel::Thread &thread : m_model->allThreads()) {
            if (thread.origin != ReviewModel::Origin::Record || !thread.open || !thread.root.author.isPerson()) continue;
            if (m_model->isOwn(thread.root)) continue;
            const QString name = thread.root.author.name;
            if (!notes.contains(name)) order << name;
            notes[name] << (thread.root.hasAt() ? QStringLiteral("%1 %2").arg(review::timecode(thread.root.at, m_fps), thread.root.body)
                                                : thread.root.body);
        }
        for (const QString &name : std::as_const(order))
            m_suggestionList << Suggestion{QStringLiteral("Address %1's open notes").arg(firstName(name)),
                                           QStringLiteral("@Kerf Address %1's open notes:\n%2").arg(name, notes.value(name).join(QLatin1Char('\n'))),
                                           false};
    }
    // Each failing check row.
    for (const QJsonValue &value : std::as_const(m_checkRows)) {
        const QJsonObject row = value.toObject();
        if (row.value(QStringLiteral("passed")).toBool(true)) continue;
        const QString name = row.value(QStringLiteral("name")).toString();
        if (name.isEmpty()) continue;
        const QString lowered = name.left(1).toLower() + name.mid(1);
        QString request = QStringLiteral("@Kerf Fix the %1").arg(lowered);
        const QJsonValue at = row.value(QStringLiteral("at"));
        if (at.isDouble()) request += QStringLiteral(" at %1").arg(review::timecode(at.toDouble(), m_fps));
        m_suggestionList << Suggestion{QStringLiteral("Fix the %1").arg(lowered), request, false};
    }
    m_suggestionList << Suggestion{QStringLiteral("Run QC on the whole sequence"), QStringLiteral("qc"), true};
    QStringList labels;
    for (const Suggestion &suggestion : std::as_const(m_suggestionList)) labels << suggestion.label;
    m_composer->setSuggestions(labels);
    m_composer->setMentionCandidates(mentionCandidates());
}

QScrollArea *ChatPanel::scroll() const { return m_list; }
QFrame *ChatPanel::composer() const { return m_composer; }
QTextEdit *ChatPanel::input() const { return m_composer->input(); }
QPushButton *ChatPanel::sendButton() const { return m_composer->sendButton(); }
QPushButton *ChatPanel::stopButton() const { return m_composer->stopButton(); }
QWidget *ChatPanel::welcome() const { return m_welcomeRow; }

QList<QAbstractButton *> ChatPanel::examples() const {
    QList<QAbstractButton *> out;
    for (SuggestionButton *button : m_welcomeRow->suggestions()) out << button;
    return out;
}

// ---- menus ----------------------------------------------------------------------------

QMenu *ChatPanel::buildSortMenu() {
    auto *menu = new QMenu(this);
    menu->setObjectName(QStringLiteral("sortMenu"));
    menu->setAttribute(Qt::WA_DeleteOnClose);
    for (const ReviewModel::Sort sort : {ReviewModel::Sort::Timecode, ReviewModel::Sort::Newest, ReviewModel::Sort::OpenFirst,
                                         ReviewModel::Sort::Commenter}) {
        QAction *action = menu->addAction(ReviewModel::sortName(sort));
        action->setCheckable(true);
        action->setChecked(m_model && m_model->sort() == sort);
        action->setEnabled(m_model);
        connect(action, &QAction::triggered, this, [this, sort] {
            if (!m_model) return;
            m_model->setSort(sort);
            updateHeader();
        });
    }
    return menu;
}

QMenu *ChatPanel::buildFilterMenu() {
    auto *menu = new QMenu(this);
    menu->setObjectName(QStringLiteral("filterMenu"));
    menu->setAttribute(Qt::WA_DeleteOnClose);
    const auto scrollTop = [this] {
        updateHeader();
        m_list->verticalScrollBar()->setValue(0);
    };
    ui::addMenuHeader(menu, QStringLiteral("From"));
    for (const auto &[name, from] : {std::pair{"Kerf", ReviewModel::From::Kerf}, std::pair{"Agents", ReviewModel::From::Agents},
                                     std::pair{"People", ReviewModel::From::People}}) {
        QAction *action = menu->addAction(QString::fromLatin1(name));
        action->setCheckable(true);
        action->setChecked(m_model && m_model->from() == from);
        connect(action, &QAction::triggered, this, [this, from = from, scrollTop] {
            if (!m_model) return;
            m_model->setFrom(m_model->from() == from ? ReviewModel::From::Anyone : from);
            scrollTop();
        });
    }
    menu->addSeparator();
    QAction *hide = menu->addAction(QStringLiteral("Hide resolved"));
    hide->setCheckable(true);
    hide->setChecked(m_model && m_model->hideResolved());
    connect(hide, &QAction::triggered, this, [this, scrollTop](bool on) {
        if (!m_model) return;
        m_model->setHideResolved(on);
        scrollTop();
    });
    QAction *drawings = menu->addAction(QStringLiteral("With drawings"));
    drawings->setCheckable(true);
    drawings->setChecked(m_model && m_model->withDrawings());
    connect(drawings, &QAction::triggered, this, [this, scrollTop](bool on) {
        if (!m_model) return;
        m_model->setWithDrawings(on);
        scrollTop();
    });
    const QList<ReviewModel::TagCount> tags = m_model ? m_model->tags() : QList<ReviewModel::TagCount>{};
    if (!tags.isEmpty()) {
        menu->addSeparator();
        ui::addMenuHeader(menu, QStringLiteral("Tags"));
        for (const ReviewModel::TagCount &tag : tags) {
            QAction *action = menu->addAction(tag.blocking ? QStringLiteral("Blocking") : QStringLiteral("#") + tag.tag);
            action->setCheckable(true);
            action->setChecked(m_model->tag() == tag.tag);
            ui::setMenuKey(action, QString::number(tag.count));
            connect(action, &QAction::triggered, this, [this, tag = tag.tag, scrollTop] {
                m_model->toggleTag(tag);
                scrollTop();
            });
        }
    }
    return menu;
}

QMenu *ChatPanel::buildMoreMenu() {
    auto *menu = new QMenu(this);
    menu->setObjectName(QStringLiteral("commentsMoreMenu"));
    menu->setAttribute(Qt::WA_DeleteOnClose);
    // Search, sort and the filter: what the header held.
    QAction *search = menu->addAction(QStringLiteral("Search comments"));
    search->setObjectName(QStringLiteral("commentsSearch"));
    ui::setMenuKey(search, QStringLiteral("⌘F"));
    connect(search, &QAction::triggered, this, [this] { setSearchShown(true); });
    QMenu *sort = buildSortMenu();
    sort->setAttribute(Qt::WA_DeleteOnClose, false);
    sort->setParent(menu, sort->windowFlags());
    sort->setTitle(QStringLiteral("Sort"));
    ui::styleMenu(sort);
    QAction *sortItem = menu->addMenu(sort);
    sortItem->setObjectName(QStringLiteral("commentsSort"));
    ui::setMenuKey(sortItem, m_model ? ReviewModel::sortName(m_model->sort()) : QString());
    menu->addSeparator();
    const ReviewModel::Counts counts = m_model ? m_model->counts() : ReviewModel::Counts{};
    const std::tuple<const char *, ReviewModel::Chip, int> chips[] = {{"All", ReviewModel::Chip::All, counts.all},
                                                                      {"Open", ReviewModel::Chip::Open, counts.open},
                                                                      {"Proposals", ReviewModel::Chip::Proposals, counts.proposals}};
    for (const auto &[name, chip, count] : chips) {
        QAction *action = menu->addAction(QString::fromLatin1(name));
        action->setObjectName(QStringLiteral("commentsShow.%1").arg(QString::fromLatin1(name)));
        action->setCheckable(true);
        action->setChecked(m_model ? m_model->chip() == chip : chip == ReviewModel::Chip::All);
        action->setEnabled(m_model);
        ui::setMenuKey(action, QString::number(count));
        connect(action, &QAction::triggered, this, [this, chip = chip] {
            if (!m_model) return;
            m_model->setChip(chip);
            updateHeader();
            m_list->verticalScrollBar()->setValue(0);
        });
    }
    QMenu *filter = buildFilterMenu();
    filter->setAttribute(Qt::WA_DeleteOnClose, false);
    filter->setParent(menu, filter->windowFlags());
    filter->setTitle(QStringLiteral("Filter"));
    ui::styleMenu(filter);
    menu->addMenu(filter)->setObjectName(QStringLiteral("commentsFilter"));
    menu->addSeparator();
    QAction *settings = menu->addAction(QStringLiteral("Agent settings…"));
    ui::setMenuKey(settings, m_currentModel);
    connect(settings, &QAction::triggered, this, [this] { emit moreAction(QStringLiteral("settings"), QString()); });
    if (m_models.size() > 1 && m_modelsChoosable) {
        QMenu *models = menu->addMenu(QStringLiteral("Model"));
        ui::styleMenu(models);
        for (const QString &model : std::as_const(m_models)) {
            QAction *action = models->addAction(model);
            action->setCheckable(true);
            action->setChecked(model == m_currentModel);
            connect(action, &QAction::triggered, this, [this, model] { emit moreAction(QStringLiteral("model"), model); });
        }
    }
    connect(menu->addAction(QStringLiteral("Activity")), &QAction::triggered, this,
            [this] { emit moreAction(QStringLiteral("activity"), QString()); });
    if (m_model) {
        for (const ReviewModel::Person &person : m_model->people()) {
            QAction *view = menu->addAction(QStringLiteral("View as %1").arg(person.name));
            view->setCheckable(true);
            view->setChecked(m_model->viewAs() == person.id && m_model->guestMode());
            connect(view, &QAction::triggered, this, [this, id = person.id](bool on) {
                emit moreAction(QStringLiteral("viewAs"), on ? id : QString());
            });
        }
    }
    menu->addSeparator();
    ui::addMenuHeader(menu, QStringLiteral("Export comments"));
    for (const auto &[label, format] : {std::pair{"As sequence markers", "markers"}, std::pair{"As CSV", "csv"},
                                        std::pair{"As EDL locators", "edl"}}) {
        QAction *action = menu->addAction(QString::fromLatin1(label));
        action->setEnabled(m_connected);
        connect(action, &QAction::triggered, this, [this, format = QString::fromLatin1(format)] {
            emit moreAction(QStringLiteral("export"), format);
        });
    }
    menu->addSeparator();
    const QString link = reviewLink(std::numeric_limits<double>::quiet_NaN());
    QAction *copy = menu->addAction(link.isEmpty() ? QStringLiteral("Copy review link") : QStringLiteral("Copy review link %1").arg(link));
    copy->setEnabled(!link.isEmpty());
    connect(copy, &QAction::triggered, this, [link] { QApplication::clipboard()->setText(link); });
    return menu;
}

QMenu *ChatPanel::buildRowMenu(const QString &commentId) {
    auto *menu = new QMenu(this);
    menu->setObjectName(QStringLiteral("rowMenu"));
    menu->setAttribute(Qt::WA_DeleteOnClose);
    menu->setMinimumWidth(240);
    const ReviewModel::Comment *comment = m_model ? m_model->comment(commentId) : nullptr;
    CommentRow *row = rowFor(commentId);
    const QString body = comment ? comment->body : row ? row->text() : QString();
    connect(menu->addAction(QStringLiteral("Copy")), &QAction::triggered, this, [body] { QApplication::clipboard()->setText(body); });
    double at = std::numeric_limits<double>::quiet_NaN();
    if (comment) {
        at = comment->at;
        if (!comment->hasAt())
            if (const ReviewModel::Thread *thread = m_model->threadOf(commentId)) at = thread->root.at;
    }
    const QString link = reviewLink(at);
    QAction *copyLink = menu->addAction(link.isEmpty() ? QStringLiteral("Copy link") : QStringLiteral("Copy link %1").arg(link));
    copyLink->setEnabled(!link.isEmpty());
    connect(copyLink, &QAction::triggered, this, [link] { QApplication::clipboard()->setText(link); });
    if (comment && comment->author.isPerson() && comment->hasAt()) {
        QAction *marker = menu->addAction(QStringLiteral("Convert to sequence marker"));
        marker->setEnabled(m_connected);
        connect(marker, &QAction::triggered, this, [this, commentId] { emit rowAction(QStringLiteral("marker"), commentId); });
    }
    if (comment && m_model->isOwn(*comment)) {
        menu->addSeparator();
        const ReviewModel::Thread *thread = m_model->threadOf(commentId);
        const QString threadId = thread ? thread->id : QString();
        QAction *again = menu->addAction(QStringLiteral("Ask again"));
        again->setEnabled(m_connected);
        connect(again, &QAction::triggered, this, [this, body, threadId] { emit askAgainInThread(body, threadId); });
        connect(menu->addAction(QStringLiteral("Edit")), &QAction::triggered, this, [this, commentId] { setEditing(commentId); });
        QAction *remove = menu->addAction(QStringLiteral("Delete"));
        ui::setMenuRole(remove, ui::MenuRole::Warn);
        remove->setEnabled(m_connected);
        connect(remove, &QAction::triggered, this, [this, commentId] { emit rowAction(QStringLiteral("delete"), commentId); });
    }
    return menu;
}

void ChatPanel::showRowMenu(CommentRow *row) {
    QMenu *menu = buildRowMenu(row->commentId());
    row->setActionsShown(true);
    connect(menu, &QMenu::aboutToHide, row, [row] { row->setActionsShown(row->underMouse()); });
    ui::popupMenu(menu, row->moreAction()->isVisible() ? static_cast<QWidget *>(row->moreAction()) : row);
}

// ---- events ------------------------------------------------------------------------------

void ChatPanel::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const QRectF area(rect());
    p.setPen(Qt::NoPen);
    p.setBrush(color(tokens::chrome));
    p.drawRect(area);
}

void ChatPanel::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    QTimer::singleShot(0, this, &ChatPanel::updatePinBar);
}

void ChatPanel::showEvent(QShowEvent *event) {
    QWidget::showEvent(event);
    if (!m_placed) QTimer::singleShot(0, this, &ChatPanel::placeFirst);
}

bool ChatPanel::eventFilter(QObject *watched, QEvent *event) {
    if (watched == m_pinBar && event->type() == QEvent::MouseButtonRelease) {
        // Opens the card and scrolls it to the list's top + 24.
        const QString id = pinBarProposal();
        if (ProposalCard *card = this->card(id)) {
            if (!card->isOpen()) {
                card->setOpen(true);
                emit card->openToggled(true);
            }
            syncContent();
            QWidget *anchor = card->parentWidget() ? card->parentWidget() : card;
            for (QWidget *up = card; up && up != m_content; up = up->parentWidget())
                if (qobject_cast<CommentRow *>(up)) {
                    anchor = up;
                    break;
                }
            m_list->verticalScrollBar()->setValue(std::max(0, anchor->mapTo(m_content, QPoint()).y() - 24));
        }
        return true;
    }
    if (watched == m_pinBar && event->type() == QEvent::Paint) {
        QPainter p(m_pinBar);
        p.fillRect(m_pinBar->rect(), color(chrome));
        p.fillRect(QRect(0, m_pinBar->height() - 1, m_pinBar->width(), 1), color(line));
        return false;
    }
    if (watched == m_search && event->type() == QEvent::KeyPress) {
        if (static_cast<QKeyEvent *>(event)->key() == Qt::Key_Escape) {
            setSearchShown(false);
            return true;
        }
    }
    return QWidget::eventFilter(watched, event);
}

} // namespace kerf
