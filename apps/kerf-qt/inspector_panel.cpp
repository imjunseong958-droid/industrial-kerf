#include "inspector_panel.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "inspector_widgets.h"
#include "media_artwork.h"

#include <QAction>
#include <QApplication>
#include <QColorDialog>
#include <QComboBox>
#include <QDragEnterEvent>
#include <QElapsedTimer>
#include <QEnterEvent>
#include <QFileDialog>
#include <QFileInfo>
#include <QFontDatabase>
#include <QFontInfo>
#include <QFrame>
#include <QHBoxLayout>
#include <QInputDialog>
#include <QKeyEvent>
#include <QLabel>
#include <QLineEdit>
#include <QMenu>
#include <QMimeData>
#include <QPainter>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QScrollArea>
#include <QScrollBar>
#include <QSettings>
#include <QSignalBlocker>
#include <QStyle>
#include <QTimer>
#include <QToolButton>
#include <QVBoxLayout>

#include <QMap>
#include <QRegularExpression>
#include <QSet>
#include <QTextDocument>

#include <algorithm>
#include <cmath>
#include <limits>
#include <tuple>

namespace kerf {

namespace {

constexpr int kSide = 16;          // the column's side padding
// The modules' plates sit this far in from the column's edges, so their
// rows (InspectorSection::kInset further in) keep the header's 16 points.
constexpr int kPlateInset = kSide - InspectorSection::kInset;
// Every value field is the same width (InspectorRow::kValueField), alone on
// its row or beside a slider or pop-up, so the column's fields line up.
constexpr int kSingleField = InspectorRow::kValueField;
constexpr int kPairedField = InspectorRow::kValueField;
// Effects show this many parameters before "N more" (N is 2 or more).
constexpr int kEffectRowsShown = 5;
// An effect's second line starts under its name: past the chevron, the
// power key and the gaps after each.
constexpr int kEffectTextIndent = 16 + 6 + 22 + 6;
const char *kFxMime = "application/x-kerf-fx";

// An open effect's "6 more ⌄": the words in 12pt/500 `text` (as Add
// effect), then a 10pt chevron that turns up once the rows are shown; a
// `hover` disc behind the chevron under the pointer. No bezel.
class MoreToggle final : public QPushButton {
public:
    static constexpr int kChevron = 10;
    static constexpr int kGap = 4;
    explicit MoreToggle(QWidget *parent = nullptr) : QPushButton(parent) {
        setAttribute(Qt::WA_Hover);
        setFlat(true);
        setCursor(Qt::PointingHandCursor);
        setFocusPolicy(Qt::TabFocus);
        QFont font = this->font();
        font.setPixelSize(12);
        font.setWeight(QFont::Medium);
        setFont(font);
        setFixedHeight(24);
    }
    void setOpen(bool open) {
        m_open = open;
        updateGeometry();
        update();
    }
    bool isOpen() const { return m_open; }
    QSize sizeHint() const override { return {fontMetrics().horizontalAdvance(text()) + kGap + kChevron + 4, 24}; }
    QSize minimumSizeHint() const override { return sizeHint(); }

protected:
    void paintEvent(QPaintEvent *) override {
        const EditorTheme &theme = editorTheme();
        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        const bool hovered = underMouse() && isEnabled();
        const int words = fontMetrics().horizontalAdvance(text());
        const QRectF chevron(words + kGap, (height() - kChevron) / 2.0, kChevron, kChevron);
        if (hovered || isDown()) {
            painter.setPen(Qt::NoPen);
            painter.setBrush(isDown() ? theme.pressed : theme.hover);
            painter.drawEllipse(chevron.center(), 8.0, 8.0);
        }
        const QColor ink = hovered ? theme.textStrong : theme.text;
        painter.setPen(ink);
        painter.drawText(QRect(0, 0, words + 1, height()), Qt::AlignLeft | Qt::AlignVCenter, text());
        painter.setPen(QPen(ink, 1.25, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        const double tip = m_open ? 3.0 : 7.0, base = m_open ? 6.5 : 3.5;
        painter.drawPolyline(QPolygonF{chevron.topLeft() + QPointF(1.5, base), chevron.topLeft() + QPointF(5.0, tip),
                                       chevron.topLeft() + QPointF(8.5, base)});
        if (hasFocus()) {
            painter.setPen(QPen(theme.focusRing, 1.0));
            painter.setBrush(Qt::NoBrush);
            painter.drawRoundedRect(QRectF(rect()).adjusted(0.5, 0.5, -0.5, -0.5), 4, 4);
        }
    }

private:
    bool m_open = false;
};

QString key(const char *text) {
    return QString::fromLatin1(text);
}

QFont sizedFont(const QFont &base, int pixels, QFont::Weight weight = QFont::Normal) {
    QFont font = base;
    font.setPixelSize(pixels);
    font.setWeight(weight);
    return font;
}

QLabel *quietLabel(const QString &text, int pixels = 12) {
    auto *label = new QLabel(text);
    label->setObjectName(QStringLiteral("inspectorQuiet"));
    label->setFont(sizedFont(label->font(), pixels));
    return label;
}

QString titleCase(const QString &text) {
    QString out = text;
    out.replace(QLatin1Char('-'), QLatin1Char(' '));
    out.replace(QLatin1Char('_'), QLatin1Char(' '));
    if (!out.isEmpty()) out[0] = out.at(0).toUpper();
    return out;
}

bool sameJson(const QJsonValue &a, const QJsonValue &b) {
    if (a.isDouble() && b.isDouble()) return std::abs(a.toDouble() - b.toDouble()) < 1e-9;
    if (a.isString() && b.isString()) return a.toString().compare(b.toString(), Qt::CaseInsensitive) == 0;
    return a == b;
}

// A pop-up or text field whose value moved from its default reads in the
// accent (the style sheet's [changed] rule), as a scrub field's does.
void markChanged(QWidget *widget, bool changed) {
    if (!widget || widget->property("changed").toBool() == changed) return;
    widget->setProperty("changed", changed);
    widget->style()->unpolish(widget);
    widget->style()->polish(widget);
}

QString ratioText(double ratio) {
    QString text = QString::number(std::abs(ratio), 'f', 4);
    while (text.endsWith(QLatin1Char('0'))) text.chop(1);
    if (text.endsWith(QLatin1Char('.'))) text.chop(1);
    return text;
}

// A generic family ("sans-serif") as the family it resolves to here
// ("Helvetica", "Inter"): what a reader recognises. Apple's private system
// family reads as "System".
QString resolvedFamily(const QString &generic) {
    QFont font(generic);
    font.setStyleHint(generic == QStringLiteral("serif") ? QFont::Serif : generic == QStringLiteral("monospace") ? QFont::Monospace
                                                                                                               : QFont::SansSerif);
    const QString family = QFontInfo(font).family();
    return family.isEmpty() || family.startsWith(QLatin1Char('.')) ? QStringLiteral("System") : family;
}

// The installed families, fetched only when the pop-up opens.
class FontPopup final : public QComboBox {
public:
    using QComboBox::QComboBox;
    void showPopup() override {
        if (!m_filled) {
            m_filled = true;
            const QString current = currentText();
            const QSignalBlocker blocker(this);
            for (const QString &family : QFontDatabase::families())
                if (findText(family) < 0) insertItem(count() - 1, family);
            setCurrentText(current);
        }
        QComboBox::showPopup();
    }

private:
    bool m_filled = false;
};

QComboBox *popup(const QString &name) {
    auto *combo = new QComboBox;
    combo->setObjectName(name);
    combo->setProperty("inspector", true);
    combo->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
    combo->setMinimumContentsLength(4);
    combo->setMinimumWidth(64);
    combo->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    return combo;
}

QPixmap swatch(const QString &color, const QSize &size) {
    QPixmap pixmap(size * 2);
    pixmap.setDevicePixelRatio(2);
    pixmap.fill(Qt::transparent);
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    // A warm grey ring that reads at 3:1 on white and black alike.
    painter.setPen(QPen(QColor(0x66, 0x5a, 0x4c), 1));
    painter.setBrush(modelColor(color));
    painter.drawRoundedRect(QRectF(QPointF(0.5, 0.5), QSizeF(size) - QSizeF(1, 1)), 2.5, 2.5);
    return pixmap;
}

} // namespace

void InspectorPanel::paintWell(QPushButton *well, const QString &color) {
    well->setIcon(swatch(color, well->iconSize()));
    well->setProperty("modelColor", color);
    well->setAccessibleDescription(color);
}

InspectorPanel::InspectorPanel(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("inspector"));
    setAttribute(Qt::WA_StyledBackground);
    setAcceptDrops(true);
    auto *outer = new QVBoxLayout(this);
    outer->setContentsMargins(0, 0, 0, 0);
    outer->setSpacing(0);
    m_header = new QWidget;
    m_header->setObjectName(QStringLiteral("inspectorHeader"));
    outer->addWidget(m_header);
    m_scroll = new QScrollArea;
    m_scroll->setObjectName(QStringLiteral("inspectorScroll"));
    // A click in the Inspector leaves the keys where they were (the
    // timeline's transport); only its fields take them, to be typed in.
    m_scroll->setFocusPolicy(Qt::NoFocus);
    m_scroll->setFrameShape(QFrame::NoFrame);
    m_scroll->setWidgetResizable(true);
    m_scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    outer->addWidget(m_scroll, 1);
    // A hairline under the header once the body has scrolled under it,
    // laid over the body's top edge rather than in the layout, so showing
    // it does not push the body down a pixel as the scroll begins.
    m_scrollEdge = new QFrame(m_scroll);
    m_scrollEdge->setObjectName(QStringLiteral("inspectorScrollEdge"));
    m_scrollEdge->setFixedHeight(1);
    m_scrollEdge->hide();
    m_scroll->installEventFilter(this);
    m_content = new QWidget;
    m_content->setObjectName(QStringLiteral("inspectorContent"));
    m_scroll->setWidget(m_content);
    m_contentLayout = new QVBoxLayout(m_content);
    m_contentLayout->setContentsMargins(kPlateInset, 0, kPlateInset, 0);
    m_contentLayout->setSpacing(0);
    m_contentLayout->addStretch(1);
    m_content->installEventFilter(this);
    // The scroll bar's showing and hiding moves the body's right padding.
    m_scroll->verticalScrollBar()->installEventFilter(this);
    m_dropLine = new QFrame(m_content);
    m_dropLine->setObjectName(QStringLiteral("inspectorDropLine"));
    m_dropLine->setFixedHeight(2);
    m_dropLine->hide();
    m_scrollBarTimer = new QTimer(this);
    m_scrollBarTimer->setSingleShot(true);
    m_scrollBarTimer->setInterval(900);
    connect(m_scrollBarTimer, &QTimer::timeout, this, [this] { showScrollBar(underMouse()); });
    // A wheel or trackpad scroll shows the scroll bar for a moment.
    m_scroll->viewport()->installEventFilter(this);
    connect(m_scroll->verticalScrollBar(), &QScrollBar::valueChanged, this, [this](int value) {
        m_scrollEdge->setVisible(value > 0);
        m_scrollEdge->raise();
        if (!m_kindForSections.isEmpty()) m_scrollByKind.insert(m_kindForSections, value);
    });
    // ⌥⌘R resets the selected clip's properties.
    auto *resetAllAction = new QAction(QStringLiteral("Reset All Properties"), this);
    resetAllAction->setObjectName(QStringLiteral("inspectorResetAll"));
    resetAllAction->setShortcut(QKeySequence(Qt::CTRL | Qt::ALT | Qt::Key_R));
    resetAllAction->setShortcutContext(Qt::WindowShortcut);
    connect(resetAllAction, &QAction::triggered, this, &InspectorPanel::resetAll);
    addAction(resetAllAction);
    buildHeader();
    connect(EditorAppearance::instance(), &EditorAppearance::changed, this, [this] {
        forgetBodies();
        refresh();
    });
    refresh();
}

InspectorPanel::~InspectorPanel() {
    delete m_parking;
}

// ------------------------------------------------------------------ state

void InspectorPanel::setState(const QJsonObject &timeline, const QStringList &clips) {
    QStringList selection = clips;
    selection.removeAll(QString());
    // A run of wheel steps on the knob edits the clips it was turned for:
    // it is sent before others take their place, not when its pause ends.
    if (selection != m_selection)
        for (RotationKnob *knob : findChildren<RotationKnob *>())
            if (knob->isWheeling()) knob->flushWheel();
    m_project = timeline;
    m_selection = selection;
    refresh();
}

void InspectorPanel::setSelection(const QStringList &clips) {
    setState(m_project, clips);
}

void InspectorPanel::setPlayhead(double seconds) {
    if (std::abs(seconds - m_playhead) < 1e-9) return;
    m_playhead = seconds;
    // Animated values follow the playhead.
    bool animated = false;
    for (const RowEntry &entry : std::as_const(m_rows)) animated |= entry.row->isAnimated();
    if (animated) refreshValues();
}

void InspectorPanel::setArtwork(MediaArtwork *artwork) {
    m_artwork = artwork;
    if (artwork) connect(artwork, &MediaArtwork::changed, this, &InspectorPanel::refreshHeader, Qt::UniqueConnection);
}

void InspectorPanel::setProgramFrame(const QImage &frame) {
    m_programFrame = frame;
    // Only the thumbnail, and only while it shows: this comes with every
    // frame played.
    if (m_mode == Mode::Sequence && m_thumbnail && (isVisible() || m_thumbnail->images().isEmpty())) m_thumbnail->setImages(thumbnails());
}

void InspectorPanel::setEffectCatalog(const QJsonArray &catalog) {
    m_effectDefs.clear();
    for (const QJsonValue &value : catalog) {
        const QJsonObject fx = value.toObject();
        m_effectDefs.insert(fx.value(QStringLiteral("id")).toString(), fx);
    }
    forgetBodies();
}

// The tracks are part of the shape (the picture tracks are the ones the
// selected clip is not on, so they change from clip to clip): a body is
// built, and parked, for each.
void InspectorPanel::setPictureTracks(const QStringList &tracks) {
    m_pictureTracks = tracks;
}

void InspectorPanel::setSoundTracks(const QStringList &tracks) {
    m_soundTracks = tracks;
}

void InspectorPanel::setMaskTools(const QList<QAction *> &tools) {
    m_maskTools = tools;
    forgetBodies();
}

void InspectorPanel::setMaskCatalog(const QJsonArray &defs, const QStringList &modes) {
    if (defs == m_maskDefs && modes == m_maskModes) return;
    m_maskDefs = defs;
    m_maskModes = modes;
    forgetBodies();
}

void InspectorPanel::setMaskable(bool maskable) {
    // Part of the shape: a body for each is built (and parked) apart.
    m_maskable = maskable;
}

void InspectorPanel::selectMask(const QString &name) {
    m_selectedMask = name;
    if (!name.isEmpty()) m_openMasks.insert(name);
    for (auto *button : findChildren<QToolButton *>(QStringLiteral("maskName"))) {
        const QSignalBlocker blocker(button);
        button->setChecked(button->text() == name);
    }
    for (QWidget *box : findChildren<QWidget *>(QStringLiteral("maskBox"))) {
        const bool open = m_openMasks.contains(box->property("mask").toString());
        if (auto *body = box->findChild<QWidget *>(QStringLiteral("maskBody"))) body->setVisible(open);
        if (auto *chevron = box->findChild<GlyphButton *>(QStringLiteral("maskChevron"))) chevron->setOpen(open);
    }
}

void InspectorPanel::revealEffect(int index) {
    m_revealEffect = index;
    refresh();
}

QString InspectorPanel::kindText() const {
    return m_kind ? m_kind->text() : QString();
}

QString InspectorPanel::nameText() const {
    return m_name ? m_name->name() : QString();
}

InspectorSection *InspectorPanel::section(const QString &id) const {
    for (const SectionEntry &entry : m_sections)
        if (entry.section->id() == id) return entry.section;
    return nullptr;
}

void InspectorPanel::gatherClips() {
    m_clips.clear();
    QHash<QString, QJsonObject> sources;
    for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray())
        sources.insert(value.toObject().value(QStringLiteral("id")).toString(), value.toObject());
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    for (const QString &id : std::as_const(m_selection)) {
        for (const QJsonValue &trackValue : tracks) {
            const QJsonObject track = trackValue.toObject();
            for (const QJsonValue &clipValue : track.value(QStringLiteral("clips")).toArray()) {
                const QJsonObject clip = clipValue.toObject();
                if (clip.value(QStringLiteral("id")).toString() != id) continue;
                Clip entry{id, clip, track, sources.value(clip.value(QStringLiteral("sourceId")).toString()), {}, {}};
                // The clip linked to it on a track of the other kind.
                const QString link = clip.value(QStringLiteral("link")).toString();
                if (!link.isEmpty()) {
                    for (const QJsonValue &otherTrack : tracks) {
                        const QJsonObject other = otherTrack.toObject();
                        if (other.value(QStringLiteral("type")) == track.value(QStringLiteral("type"))) continue;
                        for (const QJsonValue &otherClip : other.value(QStringLiteral("clips")).toArray())
                            if (otherClip.toObject().value(QStringLiteral("link")).toString() == link) {
                                entry.linked = otherClip.toObject();
                                entry.linkedTrack = other;
                            }
                    }
                }
                m_clips.append(entry);
            }
        }
    }
    if (m_clips.isEmpty()) {
        m_mode = Mode::Sequence;
        return;
    }
    if (m_clips.size() == 1) {
        m_mode = Mode::Clip;
        return;
    }
    const QString first = kindKey(m_clips.first());
    const bool same = std::all_of(m_clips.cbegin(), m_clips.cend(), [this, &first](const Clip &clip) { return kindKey(clip) == first; });
    m_mode = same ? Mode::Several : Mode::MixedKinds;
}

QString InspectorPanel::kindKey(const Clip &clip) const {
    const QString kind = clip.clip.value(QStringLiteral("kind")).toString();
    const QString type = clip.track.value(QStringLiteral("type")).toString();
    if (kind == QStringLiteral("caption") || type == QStringLiteral("subtitle")) return QStringLiteral("caption");
    if (type == QStringLiteral("audio")) return QStringLiteral("audio");
    if (kind == QStringLiteral("generator")) {
        const QString fx = clip.clip.value(QStringLiteral("fx")).toString();
        if (fx == QStringLiteral("title") || fx == QStringLiteral("solid") || fx == QStringLiteral("adjustment")) return fx;
        return QStringLiteral("generator:") + fx;
    }
    if (kind == QStringLiteral("sequence")) return QStringLiteral("sequence");
    return QStringLiteral("video");
}

QString InspectorPanel::kindName(const Clip &clip) const {
    const QString key = kindKey(clip);
    static const QHash<QString, QString> names{
        {QStringLiteral("video"), QStringLiteral("Video")},       {QStringLiteral("sequence"), QStringLiteral("Sequence")},
        {QStringLiteral("audio"), QStringLiteral("Audio")},       {QStringLiteral("title"), QStringLiteral("Title")},
        {QStringLiteral("solid"), QStringLiteral("Solid")},       {QStringLiteral("adjustment"), QStringLiteral("Adjustment")},
        {QStringLiteral("caption"), QStringLiteral("Caption")},
    };
    if (names.contains(key)) return names.value(key);
    return titleCase(clip.clip.value(QStringLiteral("fx")).toString(QStringLiteral("Generator")));
}

QString InspectorPanel::displayName(const Clip &clip) const {
    // Its own name; else its media's file name; else what generates it
    // ("Title", "Solid"); else where it is ("Clip on V1"). Never a stand-in
    // such as "Untitled media".
    const QString name = clip.clip.value(QStringLiteral("name")).toString().trimmed();
    if (!name.isEmpty() && name != QStringLiteral("Untitled media")) return name;
    const QString path = clip.source.value(QStringLiteral("path")).toString();
    if (!path.isEmpty()) return QFileInfo(path).fileName();
    if (clip.clip.value(QStringLiteral("kind")).toString() == QStringLiteral("generator")) {
        const QString fx = clip.clip.value(QStringLiteral("fx")).toString();
        const QString generator = m_effectDefs.value(fx).value(QStringLiteral("name")).toString();
        return generator.isEmpty() ? kindName(clip) : generator;
    }
    return QStringLiteral("Clip on %1").arg(trackBadge(m_project, clip.track.value(QStringLiteral("id")).toString()).badge);
}

double InspectorPanel::clipTime(const Clip &clip) const {
    return std::clamp(m_playhead - clip.clip.value(QStringLiteral("timelineStart")).toDouble(), 0.0,
                      clip.clip.value(QStringLiteral("duration")).toDouble());
}

QString InspectorPanel::shapeKey() const {
    QStringList parts{QString::number(int(m_mode)), QString::number(m_clips.size()), m_maskable ? key("m") : key("-"),
                      m_pictureTracks.join(QLatin1Char(',')), m_soundTracks.join(QLatin1Char(','))};
    if (m_mode == Mode::Sequence) {
        parts << QString::number(m_project.value(QStringLiteral("tracks")).toArray().size())
              << QString::number(m_project.value(QStringLiteral("markers")).toArray().size());
        return parts.join(QLatin1Char('|'));
    }
    for (const Clip &clip : m_clips) {
        parts << kindKey(clip) << clip.track.value(QStringLiteral("type")).toString();
        if (m_clips.size() == 1) {
            QStringList stack;
            for (const QJsonValue &effect : clip.clip.value(QStringLiteral("effects")).toArray())
                stack << effect.toObject().value(QStringLiteral("fx")).toString();
            QStringList masks;
            for (const QJsonValue &mask : clip.clip.value(QStringLiteral("masks")).toArray())
                masks << mask.toObject().value(QStringLiteral("name")).toString();
            parts << stack.join(QLatin1Char(',')) << masks.join(QLatin1Char(',')) << (clip.linked.isEmpty() ? key("") : key("linked"))
                  << (clip.clip.contains(QStringLiteral("speed")) ? key("speed") : key("")) << (clip.source.isEmpty() ? key("") : key("source"));
        }
        QStringList defs;
        for (const QJsonValue &def : defsOf(clip)) defs << def.toObject().value(QStringLiteral("name")).toString();
        parts << defs.join(QLatin1Char(','));
    }
    return parts.join(QLatin1Char('|'));
}

void InspectorPanel::refresh() {
    gatherClips();
    const QString shape = shapeKey();
    if (shape != m_shape) {
        parkBody();
        m_shape = shape;
        if (!unparkBody(shape)) rebuild();
    }
    refreshHeader();
    refreshValues();
    if (m_revealEffect >= 0 && m_revealEffect < m_effectBoxes.size()) {
        QWidget *box = m_effectBoxes.at(m_revealEffect);
        if (InspectorSection *effects = section(QStringLiteral("effects")); effects && !effects->isOpen()) {
            effects->setOpen(true);
            saveSections();
        }
        if (auto *params = box->findChild<QWidget *>(QStringLiteral("effectParams"))) params->show();
        if (auto *chevron = box->findChild<GlyphButton *>(QStringLiteral("effectChevron"))) chevron->setOpen(true);
        QSettings().setValue(QStringLiteral("editor/inspector/fx/%1/open").arg(box->property("fx").toString()), true);
        m_revealEffect = -1;
        QTimer::singleShot(0, this, [this, box = QPointer<QWidget>(box)] {
            if (box) m_scroll->ensureWidgetVisible(box, 0, 16);
        });
    }
}

void InspectorPanel::clearBody() {
    m_rows.clear();
    m_sections.clear();
    m_refreshers.clear();
    m_effectBoxes.clear();
    m_timingFields.clear();
    if (m_bodyWidget) {
        m_contentLayout->removeWidget(m_bodyWidget);
        delete m_bodyWidget;
        m_bodyWidget = nullptr;
    }
}

void InspectorPanel::rebuild() {
    QElapsedTimer cost;
    cost.start();
    clearBody();
    m_bodyWidget = new QWidget;
    m_bodyWidget->setObjectName(QStringLiteral("inspectorBody"));
    auto *body = new QVBoxLayout(m_bodyWidget);
    // The first module just clear of the glass; each keeps its own gap
    // under it (none inside a rack of folded ones).
    body->setContentsMargins(0, 2, 0, 0);
    body->setSpacing(0);
    // The header's closing hairline, which scrolls away with the body.
    auto *rule = new QFrame;
    rule->setObjectName(QStringLiteral("inspectorRule"));
    rule->setFixedHeight(1);
    body->addWidget(rule);
    m_kindForSections = m_mode == Mode::Sequence     ? QStringLiteral("sequence")
                        : m_mode == Mode::MixedKinds ? QStringLiteral("mixed")
                        : m_mode == Mode::Several    ? QStringLiteral("several-") + kindKey(m_clips.first())
                                                     : kindKey(m_clips.first());
    // The timing row, in the header: fields for clips, text for the sequence.
    buildHeader();
    if (m_mode == Mode::Sequence) {
        buildSequence(body);
    } else if (m_mode == Mode::MixedKinds) {
        rule->hide();
    } else {
        const QString kind = kindKey(m_clips.first());
        if (kind == QStringLiteral("video") || kind == QStringLiteral("sequence")) buildVideo(body, false);
        else if (kind == QStringLiteral("audio")) buildAudio(body);
        else if (kind == QStringLiteral("title")) buildTitle(body);
        else if (kind == QStringLiteral("solid")) buildSolid(body);
        else if (kind == QStringLiteral("adjustment")) buildAdjustment(body);
        else if (kind == QStringLiteral("caption")) buildCaption(body);
        else buildGenerator(body);
    }
    // A module brings its own frame: no hairline over it.
    if (QLayoutItem *next = body->itemAt(1); next && qobject_cast<InspectorSection *>(next->widget())) rule->hide();
    m_bodyCost = cost.elapsed();
    showBody();
}

void InspectorPanel::updateRack() {
    const auto folded = [this](qsizetype at) { return at >= 0 && at < m_sections.size() && !m_sections.at(at).section->isOpen(); };
    for (qsizetype index = 0; index < m_sections.size(); ++index) {
        InspectorSection *module = m_sections.at(index).section;
        module->setJoined(folded(index) && folded(index - 1), folded(index) && folded(index + 1));
    }
}

void InspectorPanel::showBody() {
    updateRack();
    m_contentLayout->insertWidget(0, m_bodyWidget);
    m_bodyWidget->show();
    // Back where the body was for this kind, once it is laid out; until
    // then the column is not repainted, so it never shows a frame scrolled
    // to the top first.
    const int offset = m_scrollByKind.value(m_kindForSections);
    m_scroll->viewport()->setUpdatesEnabled(false);
    QTimer::singleShot(0, this, [this, offset, shape = m_shape] {
        if (shape == m_shape) m_scroll->verticalScrollBar()->setValue(offset);
        m_scroll->viewport()->setUpdatesEnabled(true);
    });
}

void InspectorPanel::parkBody() {
    if (!m_bodyWidget) return;
    // A body whose shape was forgotten is out of date, and one quicker to
    // build than to move back into the column is built again: it goes.
    if (m_shape.isEmpty() || m_bodyCost < kParkedBodyCost) {
        clearBody();
        return;
    }
    if (!m_parking) m_parking = new QWidget;
    m_contentLayout->removeWidget(m_bodyWidget);
    m_bodyWidget->hide();
    m_bodyWidget->setParent(m_parking);
    m_parked.insert(m_shape, ParkedBody{m_bodyWidget, m_kindForSections, m_rows, m_sections, m_refreshers, m_effectBoxes});
    m_parkedOrder.removeAll(m_shape);
    m_parkedOrder.append(m_shape);
    while (m_parkedOrder.size() > kParkedBodies) delete m_parked.take(m_parkedOrder.takeFirst()).widget;
    m_bodyWidget = nullptr;
    clearBody();
}

bool InspectorPanel::unparkBody(const QString &shape) {
    const auto found = m_parked.constFind(shape);
    if (found == m_parked.constEnd()) return false;
    const ParkedBody parked = found.value();
    m_parked.erase(found);
    m_parkedOrder.removeAll(shape);
    clearBody();
    m_bodyWidget = parked.widget;
    m_kindForSections = parked.kind;
    m_rows = parked.rows;
    m_sections = parked.sections;
    m_refreshers = parked.refreshers;
    m_effectBoxes = parked.effectBoxes;
    // It was worth parking once, so it is again.
    m_bodyCost = kParkedBodyCost;
    buildHeader();
    showBody();
    return true;
}

void InspectorPanel::forgetBodies() {
    m_shape.clear();
    for (const ParkedBody &parked : std::as_const(m_parked)) delete parked.widget;
    m_parked.clear();
    m_parkedOrder.clear();
}

// ---------------------------------------------------------------- header

void InspectorPanel::buildHeader() {
    // The header's parts are made once; only the timing row follows the mode.
    if (!m_thumbnail) {
        auto *layout = new QVBoxLayout(m_header);
        layout->setContentsMargins(kSide, 8, kSide, 4);
        layout->setSpacing(6);
        auto *top = new QHBoxLayout;
        top->setSpacing(8);
        m_thumbnail = new InspectorThumbnail;
        top->addWidget(m_thumbnail, 0, Qt::AlignTop);
        auto *identity = new QVBoxLayout;
        identity->setSpacing(2);
        identity->setContentsMargins(0, 0, 0, 0);
        m_name = new NameLabel;
        connect(m_name, &NameLabel::renamed, this, [this](const QString &name) {
            if (m_mode != Mode::Clip) return;
            emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("set")}, {QStringLiteral("clip"), m_clips.first().id},
                                           {QStringLiteral("name"), name}}});
        });
        // The name with ⋯ at its end; under them the kind line, which runs
        // to the column's edge (under ⋯ too), so "Video · V1 Video 1 ·
        // linked A1" fits the 300 column.
        auto *nameRow = new QHBoxLayout;
        nameRow->setContentsMargins(0, 0, 0, 0);
        nameRow->setSpacing(4);
        nameRow->addWidget(m_name, 1);
        identity->addLayout(nameRow);
        auto *kindRow = new QHBoxLayout;
        kindRow->setSpacing(6);
        m_kindSquare = new KindSquare;
        kindRow->addWidget(m_kindSquare, 0, Qt::AlignVCenter);
        m_kind = new QLabel;
        m_kind->setObjectName(QStringLiteral("inspectorKind"));
        m_kind->setFont(sizedFont(m_kind->font(), 12));
        m_kind->setTextFormat(Qt::RichText);
        m_kind->setMinimumWidth(0);
        m_kind->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
        kindRow->addWidget(m_kind, 1);
        identity->addLayout(kindRow);
        top->addLayout(identity, 1);
        m_more = new GlyphButton(GlyphButton::Glyph::More);
        m_more->setObjectName(QStringLiteral("inspectorMore"));
        m_more->setFixedSize(28, 28);
        m_more->setToolTip(QStringLiteral("More"));
        m_moreMenu = new QMenu(this);
        m_moreMenu->setObjectName(QStringLiteral("inspectorMoreMenu"));
        connect(m_more, &QAbstractButton::clicked, this, [this] {
            m_moreMenu->popup(m_more->mapToGlobal(QPoint(0, m_more->height())));
        });
        nameRow->addWidget(m_more, 0, Qt::AlignVCenter);
        layout->addLayout(top);
        m_timing = new GlassReadout;
        layout->addWidget(m_timing);
        m_note = quietLabel(QString());
        m_note->setObjectName(QStringLiteral("inspectorNote"));
        m_note->setWordWrap(true);
        // Its own line on the 8pt rhythm: 8 under the timing row (the
        // layout's 6 and 2 more) and 8 over the hairline (4 and the
        // header's 4).
        m_note->setContentsMargins(0, 2, 0, 4);
        m_note->hide();
        layout->addWidget(m_note);
        return;
    }
    // The timing's fields are made again for the mode: plain figures for the sequence.
    m_timing->clearColumns();
    m_timingFields.clear();
    buildTiming();
    m_name->setEditable(m_mode == Mode::Clip);
}

void InspectorPanel::buildTiming() {
    const EditorTheme &theme = editorTheme();
    const double fps = m_project.value(QStringLiteral("fps")).toDouble(24.0);
    const bool drop = m_project.value(QStringLiteral("dropFrame")).toBool();
    // Labelled as a deck's counter: in, out and duration.
    const struct {
        const char *field;
        const char *label;
        const char *name;
    } columns[] = {{"start", "In", "Start"}, {"end", "Out", "End"}, {"duration", "Dur", "Duration"}};
    for (const auto &column : columns) {
        const QString name = QString::fromLatin1(column.name);
        if (m_mode == Mode::Sequence) {
            // Read-only: the figures on the glass, nothing to scrub.
            auto *value = new QLabel;
            value->setObjectName(QStringLiteral("timing_%1").arg(QString::fromLatin1(column.field)));
            value->setFont(monoFont(11));
            value->setStyleSheet(QStringLiteral("color: %1;").arg(theme.glassText.name()));
            value->setIndent(3);
            value->setFixedHeight(ScrubField::kHeight);
            value->setAccessibleName(name);
            m_timing->addColumn(QString::fromLatin1(column.label), value);
        } else {
            auto *field = newScrubField();
            field->setObjectName(QStringLiteral("timing_%1").arg(QString::fromLatin1(column.field)));
            field->setGlass(true);
            field->setAccessibleName(name);
            field->setToolTip(name);
            NumberFormat format;
            format.kind = NumberFormat::Kind::Timecode;
            format.fps = fps;
            format.dropFrame = drop;
            format.min = QString::fromLatin1(column.field) == QStringLiteral("duration") ? 1.0 : 0.0;
            format.max = 1e9;
            format.step = 1.0;
            format.bigStep = 10.0;
            format.smallStep = 1.0;
            field->setFormat(format);
            const QString edge = QString::fromLatin1(column.field);
            connect(field, &ScrubField::committed, this, [this, edge](double value, double delta, bool relative) {
                timingCommitted(edge, value, delta, relative);
            });
            m_timing->addColumn(QString::fromLatin1(column.label), field);
            m_timingFields << field;
        }
    }
}

QImage InspectorPanel::thumbnailOf(const Clip &clip) const {
    const EditorTheme &theme = editorTheme();
    QImage image(QSize(128, 72), QImage::Format_ARGB32_Premultiplied);
    image.fill(theme.thumbnail);
    QPainter painter(&image);
    painter.setRenderHint(QPainter::Antialiasing);
    const QString kind = kindKey(clip);
    const QRectF all(0, 0, 128, 72);
    const auto frameOf = [this](const QString &source) {
        return m_artwork ? m_artwork->frames.value(source) : QImage();
    };
    if (kind == QStringLiteral("video") || kind == QStringLiteral("sequence")) {
        const QImage frame = frameOf(clip.clip.value(QStringLiteral("sourceId")).toString());
        if (!frame.isNull()) {
            const QSizeF scaled = QSizeF(frame.size()).scaled(all.size(), Qt::KeepAspectRatioByExpanding);
            painter.drawImage(QRectF(all.center() - QPointF(scaled.width(), scaled.height()) / 2.0, scaled), frame);
        } else {
            painter.fillRect(all, theme.tileEmpty);
            painter.setPen(QPen(theme.tileEmptyText, 2.0));
            painter.drawRect(QRectF(44, 22, 40, 28));
        }
    } else if (kind == QStringLiteral("audio")) {
        // Its real waveform, as the bin's tile draws it, over the part of the
        // source the clip plays (its in to its out).
        painter.fillRect(all, theme.audioTile);
        const QString sourceId = clip.clip.value(QStringLiteral("sourceId")).toString();
        const QImage wave = m_artwork ? m_artwork->waves.value(sourceId) : QImage();
        if (!wave.isNull()) {
            const double length = clip.source.value(QStringLiteral("duration")).toDouble();
            double from = 0.0, to = 1.0;
            if (length > 0.0 && clip.clip.contains(QStringLiteral("in"))) {
                from = std::clamp(clip.clip.value(QStringLiteral("in")).toDouble() / length, 0.0, 1.0);
                to = std::clamp(clip.clip.value(QStringLiteral("out")).toDouble(length) / length, from, 1.0);
            }
            const int left = int(std::floor(from * wave.width()));
            const int right = std::max(left + 1, int(std::ceil(to * wave.width())));
            QImage ink = wave.copy(left, 0, right - left, wave.height()).convertToFormat(QImage::Format_ARGB32_Premultiplied);
            // FFmpeg draws the wave in white: ink it in the tile's glyph colour.
            QPainter tint(&ink);
            tint.setCompositionMode(QPainter::CompositionMode_SourceIn);
            tint.fillRect(ink.rect(), theme.audioTileGlyph);
            tint.end();
            painter.setRenderHint(QPainter::SmoothPixmapTransform);
            painter.drawImage(QRectF(0, 14, 128, 44), ink);
        } else {
            drawWaveformSymbol(painter, all.center(), 40, theme.audioTileGlyph);
        }
    } else if (kind == QStringLiteral("solid")) {
        painter.fillRect(all, modelColor(clip.clip.value(QStringLiteral("props")).toObject().value(QStringLiteral("color")).toString(QStringLiteral("#000000"))));
    } else if (kind == QStringLiteral("caption")) {
        painter.fillRect(all, theme.viewerCanvas);
        QFont font = painter.font();
        font.setPixelSize(14);
        painter.setFont(font);
        const QString words = clip.clip.value(QStringLiteral("text")).toString().section(QLatin1Char(' '), 0, 2);
        const QRectF box = QRectF(QPointF(8, 40), QSizeF(112, 24));
        painter.setPen(Qt::NoPen);
        painter.setBrush(theme.viewerCaption);
        painter.drawRoundedRect(box, 4, 4);
        painter.setPen(theme.viewerCaptionText);
        painter.drawText(box, Qt::AlignCenter, painter.fontMetrics().elidedText(words, Qt::ElideRight, 104));
    } else if (kind == QStringLiteral("adjustment")) {
        // What it changes: the picture under it (the Program frame), dimmed,
        // with the effect mark over it at 14 points.
        painter.fillRect(all, theme.viewerCanvas);
        if (!m_programFrame.isNull()) {
            const QSizeF scaled = QSizeF(m_programFrame.size()).scaled(all.size(), Qt::KeepAspectRatioByExpanding);
            painter.drawImage(QRectF(all.center() - QPointF(scaled.width(), scaled.height()) / 2.0, scaled), m_programFrame);
            painter.fillRect(all, QColor(0, 0, 0, 120));
        }
        QFont font = painter.font();
        font.setPixelSize(28); // 14 points in the 64 by 36 thumbnail
        font.setWeight(QFont::DemiBold);
        font.setItalic(true);
        painter.setFont(font);
        painter.setPen(QColor(0xf4, 0xec, 0xde));
        painter.drawText(all, Qt::AlignCenter, QStringLiteral("fx"));
    } else {
        // Titles and other generators: the text (or the effect symbol) on the letterbox.
        painter.fillRect(all, theme.viewerCanvas);
        QFont font = painter.font();
        font.setPixelSize(18);
        font.setWeight(QFont::DemiBold);
        painter.setFont(font);
        painter.setPen(QColor(0xf4, 0xec, 0xde));
        const QString text = kind == QStringLiteral("title")
            ? clip.clip.value(QStringLiteral("props")).toObject().value(QStringLiteral("text")).toString(clipName(clip.clip))
            : QStringLiteral("fx");
        painter.drawText(all.adjusted(6, 0, -6, 0), Qt::AlignCenter, painter.fontMetrics().elidedText(text, Qt::ElideRight, 116));
    }
    return image;
}

QList<QImage> InspectorPanel::thumbnails() const {
    QList<QImage> images;
    if (m_mode == Mode::Sequence) {
        if (!m_programFrame.isNull()) {
            images << m_programFrame;
        } else {
            QImage blank(QSize(128, 72), QImage::Format_ARGB32_Premultiplied);
            blank.fill(editorTheme().viewerCanvas);
            images << blank;
        }
        return images;
    }
    for (const Clip &clip : m_clips.mid(0, 3)) images << thumbnailOf(clip);
    return images;
}

void InspectorPanel::refreshHeader() {
    if (!m_thumbnail) return;
    const EditorTheme &theme = editorTheme();
    m_thumbnail->setImages(thumbnails());
    const auto secondary = [&theme](const QString &text) {
        return QStringLiteral("<span style=\"color:%1\">%2</span>").arg(theme.textSecondary.name(), text.toHtmlEscaped());
    };
    // As the timeline's header names the track: "V3 Captions", "V1 Video 1".
    const auto trackLabel = [this](const QJsonObject &track) {
        return trackBadge(m_project, track.value(QStringLiteral("id")).toString()).label();
    };
    const auto trackBadgeOf = [this](const QJsonObject &track) {
        return trackBadge(m_project, track.value(QStringLiteral("id")).toString()).badge;
    };
    const double fps = m_project.value(QStringLiteral("fps")).toDouble(24.0);
    const bool drop = m_project.value(QStringLiteral("dropFrame")).toBool();
    m_moreMenu->clear();
    m_note->hide();
    if (m_mode == Mode::Sequence) {
        QString name;
        const QJsonArray sequences = m_project.value(QStringLiteral("sequences")).toArray();
        for (const QJsonValue &sequence : sequences)
            if (sequence.toObject().value(QStringLiteral("id")) == m_project.value(QStringLiteral("sequence")))
                name = sequence.toObject().value(QStringLiteral("name")).toString();
        if (name.isEmpty()) name = m_project.value(QStringLiteral("name")).toString(QStringLiteral("Sequence"));
        m_name->setName(name);
        m_kindSquare->hide();
        const QSize size = resolvedCanvasSize(m_project, m_project.value(QStringLiteral("sources")).toArray());
        m_kind->setText(QStringLiteral("Sequence") + secondary(QStringLiteral(" · %1 × %2 · %3 fps").arg(size.width()).arg(size.height())
                                                                    .arg(QString::number(fps, 'g', 4))));
        const double start = m_project.value(QStringLiteral("start")).toDouble();
        double duration = m_project.value(QStringLiteral("duration")).toDouble();
        // The end of the last clip, if the snapshot leaves it out.
        if (!m_project.contains(QStringLiteral("duration")))
            for (const QJsonValue &track : m_project.value(QStringLiteral("tracks")).toArray())
                for (const QJsonValue &clip : track.toObject().value(QStringLiteral("clips")).toArray())
                    duration = std::max(duration, clip.toObject().value(QStringLiteral("timelineStart")).toDouble() +
                                                      clip.toObject().value(QStringLiteral("duration")).toDouble());
        const auto tc = [fps, drop](double seconds) { return frameTimecode(std::llround(seconds * fps), fps, drop); };
        if (auto *label = m_timing->findChild<QLabel *>(QStringLiteral("timing_start"))) label->setText(tc(start));
        if (auto *label = m_timing->findChild<QLabel *>(QStringLiteral("timing_end"))) label->setText(tc(start + duration));
        if (auto *label = m_timing->findChild<QLabel *>(QStringLiteral("timing_duration"))) label->setText(tc(duration));
        m_note->setText(QStringLiteral("Select a clip to adjust it."));
        m_note->show();
        m_moreMenu->addAction(QStringLiteral("Ask Kerf about the Sequence"), this, [this] { askAbout(QStringLiteral("the sequence")); })
            ->setObjectName(QStringLiteral("inspectorAsk"));
        return;
    }
    m_kindSquare->show();
    const Clip &first = m_clips.first();
    m_kindSquare->setColor(theme.clip(clipKind(first.track, first.clip)).header);
    if (m_mode == Mode::Clip) {
        m_name->setName(displayName(first));
        QString detail = QStringLiteral(" · ") + trackLabel(first.track);
        if (!first.linked.isEmpty()) detail += QStringLiteral(" · linked ") + trackBadgeOf(first.linkedTrack);
        if (!first.clip.value(QStringLiteral("enabled")).toBool(true)) detail += QStringLiteral(" · Off");
        m_kind->setText(kindName(first).toHtmlEscaped() + secondary(detail));
        const QString kind = kindKey(first);
        if (kind == QStringLiteral("adjustment")) {
            m_note->setText(QStringLiteral("Applies its effects to every layer beneath it."));
            m_note->show();
        }
        m_moreMenu->addAction(QStringLiteral("Ask Kerf about This Clip"), this, [this] { askAbout(QLatin1Char('@') + displayName(m_clips.first())); })
            ->setObjectName(QStringLiteral("inspectorAsk"));
        m_moreMenu->addAction(QStringLiteral("Rename"), m_name, &NameLabel::beginEditing)->setObjectName(QStringLiteral("inspectorRename"));
        if (!first.source.isEmpty())
            m_moreMenu->addAction(QStringLiteral("Reveal in Project"), this, [this] {
                emit revealRequested(m_clips.first().clip.value(QStringLiteral("sourceId")).toString());
            })->setObjectName(QStringLiteral("inspectorReveal"));
    } else {
        m_name->setName(QStringLiteral("%1 clips").arg(m_clips.size()));
        QMap<QString, int> kinds;
        QSet<QString> trackIds;
        for (const Clip &clip : m_clips) {
            kinds[kindName(clip)] += 1;
            trackIds.insert(trackBadgeOf(clip.track));
        }
        if (kinds.size() == 1) {
            m_kind->setText(kinds.firstKey().toHtmlEscaped() +
                            (trackIds.size() == 1 ? secondary(QStringLiteral(" · ") + *trackIds.cbegin()) : QString()));
        } else {
            QStringList parts;
            for (auto it = kinds.cbegin(); it != kinds.cend(); ++it) parts << QStringLiteral("%1 %2").arg(it.value()).arg(it.key());
            m_kind->setText(parts.join(QStringLiteral(", ")).toHtmlEscaped());
        }
        if (m_mode == Mode::MixedKinds) {
            const bool sound = std::any_of(m_clips.cbegin(), m_clips.cend(), [this](const Clip &c) { return kindKey(c) == QStringLiteral("audio"); });
            m_note->setText(sound ? QStringLiteral("Video and audio clips have no properties in common.")
                                  : QStringLiteral("These clips have no properties in common."));
            m_note->show();
        }
        m_moreMenu->addAction(QStringLiteral("Ask Kerf about These Clips"), this, [this] {
            QStringList names;
            for (const Clip &clip : std::as_const(m_clips)) names << QLatin1Char('@') + displayName(clip);
            askAbout(names.join(QLatin1Char(' ')));
        })->setObjectName(QStringLiteral("inspectorAsk"));
    }
    m_moreMenu->addSeparator();
    QAction *reset = m_moreMenu->addAction(QStringLiteral("Reset All Properties"), this, &InspectorPanel::resetAll);
    reset->setObjectName(QStringLiteral("inspectorResetAllItem"));
    reset->setShortcut(QKeySequence(Qt::CTRL | Qt::ALT | Qt::Key_R));
    reset->setShortcutVisibleInContextMenu(true);
    // The timing: the clip's, or the span of several.
    qint64 start = std::numeric_limits<qint64>::max();
    qint64 end = 0;
    QSet<qint64> durations;
    for (const Clip &clip : m_clips) {
        const qint64 s = qint64(clip.clip.value(QStringLiteral("startFrame")).toDouble(std::round(clip.clip.value(QStringLiteral("timelineStart")).toDouble() * fps)));
        const qint64 d = qint64(clip.clip.value(QStringLiteral("durationFrames")).toDouble(std::round(clip.clip.value(QStringLiteral("duration")).toDouble() * fps)));
        start = std::min(start, s);
        end = std::max(end, s + d);
        durations.insert(d);
    }
    if (m_timingFields.size() == 3) {
        m_timingFields.at(0)->setValue(double(start));
        m_timingFields.at(1)->setValue(double(end));
        m_timingFields.at(2)->setValue(double(*durations.cbegin()));
        m_timingFields.at(2)->setMixed(durations.size() > 1);
    }
}

void InspectorPanel::timingCommitted(const QString &field, double value, double delta, bool relative) {
    const double fps = m_project.value(QStringLiteral("fps")).toDouble(24.0);
    QJsonArray ops;
    const bool several = m_clips.size() > 1;
    const auto startOf = [fps](const Clip &clip) {
        return qint64(clip.clip.value(QStringLiteral("startFrame")).toDouble(std::round(clip.clip.value(QStringLiteral("timelineStart")).toDouble() * fps)));
    };
    const auto durationOf = [fps](const Clip &clip) {
        return qint64(clip.clip.value(QStringLiteral("durationFrames")).toDouble(std::round(clip.clip.value(QStringLiteral("duration")).toDouble() * fps)));
    };
    // Several clips' End is their span's end: moving it trims only the
    // clip (or clips) ending there, so the cuts inside the span stay shut.
    qint64 spanEnd = 0;
    for (const Clip &clip : std::as_const(m_clips)) spanEnd = std::max(spanEnd, startOf(clip) + durationOf(clip));
    for (const Clip &clip : std::as_const(m_clips)) {
        const qint64 start = startOf(clip);
        const qint64 duration = durationOf(clip);
        if (several && field == QStringLiteral("end") && start + duration != spanEnd) continue;
        qint64 frames = 0;
        QString op;
        QString edge;
        if (field == QStringLiteral("start")) {
            op = QStringLiteral("move");
            frames = several || relative ? std::llround(delta) : std::llround(value) - start;
        } else if (field == QStringLiteral("end")) {
            op = QStringLiteral("trim");
            edge = QStringLiteral("out");
            frames = several || relative ? std::llround(delta) : std::llround(value) - (start + duration);
        } else {
            op = QStringLiteral("trim");
            edge = QStringLiteral("out");
            frames = relative ? std::llround(delta) : std::llround(value) - duration;
        }
        if (frames == 0) continue;
        QJsonObject edit{{QStringLiteral("op"), op}, {QStringLiteral("clip"), clip.id}, {QStringLiteral("frames"), frames}};
        if (!edge.isEmpty()) edit.insert(QStringLiteral("edge"), edge);
        ops.append(edit);
    }
    emitOps(ops);
}

void InspectorPanel::askAbout(const QString &subject) {
    emit askRequested(subject);
}

void InspectorPanel::resetAll() {
    if (m_clips.isEmpty()) return;
    QJsonArray ops;
    for (const Clip &clip : std::as_const(m_clips)) {
        // The track type's own properties; a generator's text and colour stay.
        const QJsonArray defs = m_project.value(QStringLiteral("properties")).toObject().value(clip.track.value(QStringLiteral("type")).toString()).toArray();
        const QJsonObject props = clip.clip.value(QStringLiteral("props")).toObject();
        QJsonObject cleared;
        for (const QJsonValue &def : defs) {
            const QString name = def.toObject().value(QStringLiteral("name")).toString();
            if (props.contains(name)) cleared.insert(name, QJsonValue());
        }
        if (!cleared.isEmpty())
            ops.append(QJsonObject{{QStringLiteral("op"), QStringLiteral("set")}, {QStringLiteral("clip"), clip.id}, {QStringLiteral("props"), cleared}});
    }
    emitOps(ops);
}

// ------------------------------------------------------- values and edits

QJsonArray InspectorPanel::defsOf(const Clip &clip) const {
    QJsonArray defs = m_project.value(QStringLiteral("generators")).toObject().value(clip.clip.value(QStringLiteral("fx")).toString()).toArray();
    if (clip.clip.value(QStringLiteral("kind")).toString() != QStringLiteral("generator")) defs = {};
    for (const QJsonValue &def : m_project.value(QStringLiteral("properties")).toObject().value(clip.track.value(QStringLiteral("type")).toString()).toArray())
        defs.append(def);
    return defs;
}

QJsonObject InspectorPanel::defNamed(const Clip &clip, const QString &name) const {
    for (const QJsonValue &def : defsOf(clip))
        if (def.toObject().value(QStringLiteral("name")).toString() == name) return def.toObject();
    return {};
}

QJsonObject InspectorPanel::effectParamDef(int effect, const QString &name) const {
    if (m_clips.isEmpty()) return {};
    const QString fx = m_clips.first().clip.value(QStringLiteral("effects")).toArray().at(effect).toObject().value(QStringLiteral("fx")).toString();
    for (const QJsonValue &def : m_effectDefs.value(fx).value(QStringLiteral("params")).toArray())
        if (def.toObject().value(QStringLiteral("name")).toString() == name) return def.toObject();
    return {};
}

PresentationContext InspectorPanel::contextOf(const Clip &clip) const {
    PresentationContext context;
    const QSize size = resolvedCanvasSize(m_project, m_project.value(QStringLiteral("sources")).toArray());
    context.sequenceWidth = size.width();
    context.sequenceHeight = size.height();
    context.sourceWidth = clip.source.value(QStringLiteral("width")).toDouble();
    context.sourceHeight = clip.source.value(QStringLiteral("height")).toDouble();
    return context;
}

NumberFormat InspectorPanel::formatFor(const Clip &clip, const QJsonObject &def) const {
    return presentationFor(def, contextOf(clip));
}

bool InspectorPanel::targets(const Clip &clip, const Target &target) const {
    switch (target.kind) {
    case Target::Kind::Prop:
        return true;
    case Target::Kind::Linked:
        return !clip.linked.isEmpty();
    case Target::Kind::Effect:
        return target.index >= 0 && target.index < clip.clip.value(QStringLiteral("effects")).toArray().size();
    case Target::Kind::Mask:
        for (const QJsonValue &mask : clip.clip.value(QStringLiteral("masks")).toArray())
            if (mask.toObject().value(QStringLiteral("name")).toString() == target.mask) return true;
        return false;
    }
    return false;
}

QJsonValue InspectorPanel::rawValue(const Clip &clip, const Target &target) const {
    switch (target.kind) {
    case Target::Kind::Prop:
        return clip.clip.value(QStringLiteral("props")).toObject().value(target.name);
    case Target::Kind::Linked:
        return clip.linked.value(QStringLiteral("props")).toObject().value(target.name);
    case Target::Kind::Effect:
        return clip.clip.value(QStringLiteral("effects")).toArray().at(target.index).toObject().value(QStringLiteral("params")).toObject().value(target.name);
    case Target::Kind::Mask:
        for (const QJsonValue &mask : clip.clip.value(QStringLiteral("masks")).toArray())
            if (mask.toObject().value(QStringLiteral("name")).toString() == target.mask)
                return mask.toObject().value(QStringLiteral("params")).toObject().value(target.name);
        return {};
    }
    return {};
}

bool InspectorPanel::isAnimated(const Clip &clip, const Target &target) const {
    return rawValue(clip, target).isArray();
}

QJsonValue InspectorPanel::valueOf(const Clip &clip, const Target &target, const QJsonObject &def) const {
    const QJsonValue raw = rawValue(clip, target);
    if (raw.isUndefined() || raw.isNull()) return def.value(QStringLiteral("default"));
    if (raw.isArray()) {
        const QJsonArray keys = raw.toArray();
        if (def.value(QStringLiteral("type")).toString() == QStringLiteral("number"))
            return propertyAt(QJsonObject{{QStringLiteral("v"), raw}}, QStringLiteral("v"), clipTime(clip), def.value(QStringLiteral("default")).toDouble());
        // Other animated values hold: the last key at or before the playhead.
        QJsonValue held = keys.first().toObject().value(QStringLiteral("value"));
        for (const QJsonValue &k : keys)
            if (k.toObject().value(QStringLiteral("at")).toDouble() <= clipTime(clip)) held = k.toObject().value(QStringLiteral("value"));
        return held;
    }
    return raw;
}

double InspectorPanel::displayOf(const Clip &clip, const Target &target, const QJsonObject &def) const {
    return valueOf(clip, target, def).toDouble() * formatFor(clip, def).scale;
}

QJsonObject InspectorPanel::opFor(const Clip &clip, const Target &target, const QJsonValue &value) const {
    switch (target.kind) {
    case Target::Kind::Prop:
        return {{QStringLiteral("op"), QStringLiteral("set")}, {QStringLiteral("clip"), clip.id},
                {QStringLiteral("props"), QJsonObject{{target.name, value}}}};
    case Target::Kind::Linked:
        return {{QStringLiteral("op"), QStringLiteral("set")}, {QStringLiteral("clip"), clip.linked.value(QStringLiteral("id")).toString()},
                {QStringLiteral("props"), QJsonObject{{target.name, value}}}};
    case Target::Kind::Effect:
        return {{QStringLiteral("op"), QStringLiteral("fx-set")}, {QStringLiteral("clip"), clip.id}, {QStringLiteral("index"), target.index},
                {QStringLiteral("params"), QJsonObject{{target.name, value}}}};
    case Target::Kind::Mask:
        return {{QStringLiteral("op"), QStringLiteral("mask-set")}, {QStringLiteral("clip"), clip.id}, {QStringLiteral("name"), target.mask},
                {QStringLiteral("params"), QJsonObject{{target.name, value}}}};
    }
    return {};
}

void InspectorPanel::emitOps(const QJsonArray &ops) {
    if (!ops.isEmpty()) emit editRequested(ops);
}

void InspectorPanel::commitNumber(const Target &target, const QJsonObject &def, double value, double delta, bool relative) {
    QJsonArray ops;
    const bool several = m_clips.size() > 1;
    for (const Clip &clip : std::as_const(m_clips)) {
        if (!targets(clip, target) || isAnimated(clip, target)) continue;
        const NumberFormat format = formatFor(clip, def);
        const double current = displayOf(clip, target, def);
        // Several clips move alike from their own values; one takes the value.
        const double display = std::clamp(several && relative ? current + delta : value, format.min, format.max);
        if (formatNumber(format, display) == formatNumber(format, current)) continue;
        const double model = display / format.scale;
        const double fallback = def.value(QStringLiteral("default")).toDouble();
        const bool isDefault = formatNumber(format, display) == formatNumber(format, fallback * format.scale);
        ops.append(opFor(clip, target, isDefault ? QJsonValue() : QJsonValue(model)));
    }
    emitOps(ops);
}

void InspectorPanel::commitValue(const Target &target, const QJsonObject &def, const QJsonValue &value) {
    QJsonArray ops;
    for (const Clip &clip : std::as_const(m_clips)) {
        if (!targets(clip, target) || isAnimated(clip, target)) continue;
        if (sameJson(valueOf(clip, target, def), value)) continue;
        ops.append(opFor(clip, target, sameJson(value, def.value(QStringLiteral("default"))) ? QJsonValue() : value));
    }
    emitOps(ops);
}

QJsonArray InspectorPanel::resetOpsFor(const Target &target, const QJsonObject &) const {
    QJsonArray ops;
    for (const Clip &clip : m_clips) {
        if (!targets(clip, target) || isAnimated(clip, target)) continue;
        const QJsonValue raw = rawValue(clip, target);
        if (raw.isUndefined() || raw.isNull()) continue;
        ops.append(opFor(clip, target, QJsonValue()));
    }
    return ops;
}

// ------------------------------------------------------- rows and sections

int InspectorPanel::addSection(QVBoxLayout *into, const QString &id, const QString &title, bool openByDefault) {
    auto *widget = new InspectorSection(id, title);
    into->addWidget(widget);
    widget->setOpen(savedOpen(id, openByDefault));
    connect(widget, &InspectorSection::toggled, this, [this, id](bool open, Qt::KeyboardModifiers modifiers) {
        setOpenSections(id, open, modifiers);
    });
    connect(widget, &InspectorSection::foldChanged, this, &InspectorPanel::updateRack);
    const int index = int(m_sections.size());
    widget->setResetHandler([this, index] {
        QJsonArray ops;
        for (const int rowIndex : m_sections.at(index).rows)
            for (const Binding &binding : m_rows.at(rowIndex).values)
                if (binding.resetOps)
                    for (const QJsonValue &op : binding.resetOps()) ops.append(op);
        emitOps(ops);
    });
    m_sections.append(SectionEntry{widget, {}, {}});
    return index;
}

QVBoxLayout *InspectorPanel::sectionBody(int section) const {
    return m_sections.at(section).section->body();
}

void InspectorPanel::setOpenSections(const QString &id, bool open, Qt::KeyboardModifiers modifiers) {
    if (modifiers & (Qt::AltModifier | Qt::ControlModifier)) {
        for (const SectionEntry &entry : std::as_const(m_sections)) {
            if (entry.section->id() == id) continue;
            // ⌥: all alike; ⌘: only this one open.
            entry.section->setOpen((modifiers & Qt::AltModifier) ? open : false);
        }
        if (modifiers & Qt::ControlModifier) section(id)->setOpen(true);
    }
    saveSections();
}

namespace {
// Each module's fold as last left, by id, per kind of clip: a module not
// yet seen in a kind is missing, and opens or folds as it does by default.
QString foldsKey(const QString &kind) {
    return QStringLiteral("editor/inspector/folds/%1").arg(kind);
}
// Before, only the list of the modules left open, so a missing id meant
// folded, whether it was folded by hand or added since.
QString legacySectionsKey(const QString &kind) {
    return QStringLiteral("editor/inspector/sections/%1").arg(kind);
}
} // namespace

bool InspectorPanel::savedOpen(const QString &id, bool openByDefault) const {
    QSettings settings;
    const QVariantMap folds = settings.value(foldsKey(m_kindForSections)).toMap();
    if (const auto found = folds.constFind(id); found != folds.cend()) return found->toBool();
    const QVariant legacy = settings.value(legacySectionsKey(m_kindForSections));
    if (!legacy.isValid()) return openByDefault;
    // From the old list: what was left open stays open, and a module open
    // by default that is missing from it was folded by hand. The modules
    // that came open by default with the hardware finish were folded
    // before it, or not there at all, so they take their new default.
    static const QStringList kNewlyOpen{QStringLiteral("transform"), QStringLiteral("compositing"), QStringLiteral("mix"),
                                        QStringLiteral("text"), QStringLiteral("caption"), QStringLiteral("generator")};
    if (legacy.toStringList().contains(id)) return true;
    return openByDefault && kNewlyOpen.contains(id);
}

void InspectorPanel::saveSections() {
    // Merged into what is saved, so the modules of this kind's other
    // shapes keep theirs.
    QSettings settings;
    QVariantMap folds = settings.value(foldsKey(m_kindForSections)).toMap();
    for (const SectionEntry &entry : std::as_const(m_sections)) folds.insert(entry.section->id(), entry.section->isOpen());
    settings.setValue(foldsKey(m_kindForSections), folds);
}

int InspectorPanel::addRow(QVBoxLayout *into, const QString &label, int section) {
    auto *widget = new InspectorRow(label);
    into->addWidget(widget);
    const int index = int(m_rows.size());
    widget->setResetHandler([this, index] {
        QJsonArray ops;
        for (const Binding &binding : m_rows.at(index).values)
            if (binding.resetOps)
                for (const QJsonValue &op : binding.resetOps()) ops.append(op);
        emitOps(ops);
    });
    m_rows.append(RowEntry{widget, {}});
    if (section >= 0) m_sections[section].rows.append(index);
    return index;
}

ScrubField *InspectorPanel::bindNumber(int rowIndex, QHBoxLayout *into, const Target &target, const QJsonObject &def,
                                       bool withSlider, bool primaryField) {
    auto *field = newScrubField();
    InspectorSlider *slider = nullptr;
    if (withSlider) {
        slider = new InspectorSlider;
        slider->setObjectName(QStringLiteral("slider_") + target.name);
        into->addWidget(slider, 1);
        // The field is the column's one width, whatever its unit, so the
        // sliders' ends line up.
        field->setFixedWidth(kPairedField);
        into->addWidget(field);
    } else {
        into->addWidget(field, 1);
    }
    InspectorRow *r = row(rowIndex);
    if (primaryField) r->label()->setField(field);
    const QString label = presentationLabel(def);
    Binding binding;
    binding.refresh = [this, field, slider, target, def, r] {
        QList<double> values;
        QStringList shown;
        bool animated = false;
        int keys = 0;
        NumberFormat format;
        bool have = false;
        for (const Clip &clip : std::as_const(m_clips)) {
            if (!targets(clip, target)) continue;
            const NumberFormat f = formatFor(clip, def);
            if (!have) format = f;
            have = true;
            const double display = displayOf(clip, target, def);
            values << display;
            shown << formatNumber(f, display);
            if (isAnimated(clip, target)) {
                animated = true;
                keys = int(rawValue(clip, target).toArray().size());
            }
        }
        if (!have) return;
        const QString prefix = field->property("prefix").toString();
        if (!prefix.isEmpty()) format.prefix = prefix;
        if (field->property("noPrefix").toBool()) format.prefix.clear();
        field->setFormat(format);
        const double fallback = def.value(QStringLiteral("default")).toDouble() * format.scale;
        field->setDefaultValue(fallback);
        field->setValue(values.first());
        const bool mixed = shown.size() > 1 && shown.count(shown.first()) != shown.size();
        field->setMixed(mixed);
        field->setReadOnly(animated);
        if (animated)
            r->setAnimated(true, QStringLiteral("Animated, %1 keyframes. Ask Kerf or edit the script to change them.").arg(keys));
        if (slider) {
            slider->setRange(format.min, format.max);
            slider->setGainTravel(format.kind == NumberFormat::Kind::Gain);
            const bool bipolar = format.min < 0.0 && format.max > 0.0;
            slider->setOrigin(bipolar ? 0.0 : fallback);
            slider->setDetent(fallback);
            slider->setValue(values.first());
            slider->setMixed(mixed);
            slider->setEnabled(!animated);
        }
    };
    binding.changed = [this, target, def] {
        for (const Clip &clip : m_clips) {
            if (!targets(clip, target) || isAnimated(clip, target)) continue;
            const NumberFormat f = formatFor(clip, def);
            if (formatNumber(f, displayOf(clip, target, def)) != formatNumber(f, def.value(QStringLiteral("default")).toDouble() * f.scale)) return true;
        }
        return false;
    };
    binding.summary = [this, target, def, label, field] {
        for (const Clip &clip : m_clips) {
            if (!targets(clip, target) || isAnimated(clip, target)) continue;
            const NumberFormat f = formatFor(clip, def);
            const double display = displayOf(clip, target, def);
            if (formatNumber(f, display) == formatNumber(f, def.value(QStringLiteral("default")).toDouble() * f.scale)) return QString();
            const QString prefix = field->property("prefix").toString();
            const QString name = !prefix.isEmpty() ? prefix : !f.prefix.isEmpty() ? f.prefix : label;
            // A folded summary reads in whole pixels ("Y 538 px").
            NumberFormat shown = f;
            if (shown.unit == QStringLiteral("px")) shown.decimals = 0;
            return QStringLiteral("%1 %2").arg(name, formatWithUnit(shown, display));
        }
        return QString();
    };
    binding.resetOps = [this, target, def] { return resetOpsFor(target, def); };
    m_rows[rowIndex].values.append(binding);
    connect(field, &ScrubField::committed, this, [this, target, def](double value, double delta, bool relative) {
        commitNumber(target, def, value, delta, relative);
    });
    if (slider) {
        connect(slider, &InspectorSlider::moved, field, [field](double value) {
            field->setValue(value);
        });
        connect(slider, &InspectorSlider::released, this, [this, field, target, def](double value, double start) {
            field->setValue(value);
            commitNumber(target, def, value, value - start, m_clips.size() > 1);
        });
    }
    return field;
}

QWidget *InspectorPanel::bindValue(int rowIndex, QHBoxLayout *into, const Target &target, const QJsonObject &def) {
    const QString type = def.value(QStringLiteral("type")).toString();
    const QString label = presentationLabel(def);
    QWidget *editor = nullptr;
    // The pop-up or text field that shows a changed value in the accent.
    QWidget *inked = nullptr;
    std::function<void(const QJsonValue &)> show;
    if (type == QStringLiteral("enum") || type == QStringLiteral("track") || type == QStringLiteral("sound") ||
        type == QStringLiteral("mask")) {
        auto *combo = popup(QString());
        QStringList options;
        if (type == QStringLiteral("enum")) {
            for (const QJsonValue &option : def.value(QStringLiteral("options")).toArray()) options << option.toString();
        } else {
            options << QString();
            if (type == QStringLiteral("track")) options << m_pictureTracks;
            else if (type == QStringLiteral("sound")) options << m_soundTracks;
            else
                for (const QJsonValue &mask : m_clips.value(0).clip.value(QStringLiteral("masks")).toArray())
                    options << mask.toObject().value(QStringLiteral("name")).toString();
        }
        for (const QString &option : options) combo->addItem(type == QStringLiteral("enum") ? titleCase(option) : option, option);
        connect(combo, &QComboBox::activated, this, [this, combo, target, def](int index) {
            commitValue(target, def, combo->itemData(index).toString());
        });
        show = [combo](const QJsonValue &value) {
            const QSignalBlocker blocker(combo);
            const int index = combo->findData(value.toString(), Qt::UserRole, Qt::MatchFixedString);
            combo->setCurrentIndex(std::max(0, index));
        };
        editor = combo;
        inked = combo;
    } else if (type == QStringLiteral("bool")) {
        auto *toggle = new InspectorSwitch;
        connect(toggle, &QAbstractButton::clicked, this, [this, target, def](bool on) { commitValue(target, def, on); });
        show = [toggle](const QJsonValue &value) {
            const QSignalBlocker blocker(toggle);
            toggle->setChecked(value.toBool());
        };
        auto *holder = new QWidget;
        auto *holderLayout = new QHBoxLayout(holder);
        holderLayout->setContentsMargins(0, 0, 0, 0);
        holderLayout->addWidget(toggle);
        holderLayout->addStretch(1);
        into->addWidget(holder, 1);
        toggle->setObjectName(QStringLiteral("value_") + target.name);
        editor = toggle;
    } else if (type == QStringLiteral("color")) {
        auto *holder = new QWidget;
        auto *holderLayout = new QHBoxLayout(holder);
        holderLayout->setContentsMargins(0, 0, 0, 0);
        holderLayout->setSpacing(8);
        auto *well = new QPushButton;
        well->setProperty("colorWell", true);
        well->setFixedSize(44, 22);
        well->setIconSize(QSize(32, 12));
        auto *hex = new QLabel;
        hex->setObjectName(QStringLiteral("colorHex"));
        hex->setFont(sizedFont(hex->font(), 12));
        holderLayout->addWidget(well);
        holderLayout->addWidget(hex);
        holderLayout->addStretch(1);
        connect(well, &QPushButton::clicked, this, [this, well, label, target, def] {
            const QColor color = QColorDialog::getColor(modelColor(well->property("modelColor").toString()), this, label,
                                                        QColorDialog::ShowAlphaChannel);
            if (color.isValid()) commitValue(target, def, modelColorName(color));
        });
        show = [well, hex](const QJsonValue &value) {
            paintWell(well, value.toString());
            hex->setText(value.toString().toUpper());
        };
        into->addWidget(holder, 1);
        well->setObjectName(QStringLiteral("value_") + target.name);
        editor = well;
    } else if (type == QStringLiteral("file")) {
        auto *holder = new QWidget;
        auto *holderLayout = new QHBoxLayout(holder);
        holderLayout->setContentsMargins(0, 0, 0, 0);
        holderLayout->setSpacing(8);
        auto *line = new QLineEdit;
        line->setProperty("inspector", true);
        holderLayout->addWidget(line, 1);
        QStringList patterns;
        for (const QJsonValue &extension : def.value(QStringLiteral("extensions")).toArray())
            patterns << QStringLiteral("*.%1").arg(extension.toString());
        const QString filter = QStringLiteral("%1 (%2)").arg(label, patterns.join(QLatin1Char(' ')));
        // A folder glyph inside the field's right end opens the file dialog:
        // no push button beside it.
        QAction *choose = line->addAction(glyphIcon(QSize(16, 16), [](QPainter &p, const QColor &color) {
            drawInspectorGlyph(p, GlyphButton::Glyph::Folder, QRectF(0, 0, 16, 16), color);
        }), QLineEdit::TrailingPosition);
        choose->setObjectName(QStringLiteral("chooseFile"));
        choose->setToolTip(QStringLiteral("Choose a file"));
        connect(choose, &QAction::triggered, this, [this, line, label, filter, target, def] {
            const QString path = QFileDialog::getOpenFileName(this, label, line->property("path").toString(), filter);
            if (!path.isEmpty()) commitValue(target, def, path);
        });
        // The file's name only ("harbor-dusk.cube"), its whole path in the
        // tooltip; typing a new path replaces it, the name left as shown
        // keeps it.
        connect(line, &QLineEdit::editingFinished, this, [this, line, target, def] {
            if (line->text() == line->property("shown").toString()) return;
            commitValue(target, def, line->text());
        });
        show = [line](const QJsonValue &value) {
            const QSignalBlocker blocker(line);
            const QString path = value.toString();
            const QString name = QFileInfo(path).fileName().isEmpty() ? path : QFileInfo(path).fileName();
            line->setProperty("path", path);
            line->setProperty("shown", name);
            line->setText(name);
            line->setCursorPosition(0);
            line->setToolTip(path);
        };
        editor = holder;
        inked = line;
        into->addWidget(holder, 1);
    } else {
        auto *line = new QLineEdit;
        line->setProperty("inspector", true);
        connect(line, &QLineEdit::editingFinished, this, [this, line, target, def] { commitValue(target, def, line->text()); });
        show = [line](const QJsonValue &value) {
            if (line->hasFocus()) return;
            const QSignalBlocker blocker(line);
            line->setText(value.toString());
        };
        editor = line;
        inked = line;
    }
    if (editor->parentWidget() == nullptr) into->addWidget(editor, 1);
    QString tip = def.value(QStringLiteral("doc")).toString();
    editor->setToolTip(tip.trimmed());
    InspectorRow *r = row(rowIndex);
    Binding binding;
    binding.refresh = [this, show, target, def, editor, inked, r] {
        bool animated = false;
        bool changed = false;
        for (const Clip &clip : std::as_const(m_clips)) {
            if (!targets(clip, target)) continue;
            const QJsonValue value = valueOf(clip, target, def);
            show(value);
            animated = isAnimated(clip, target);
            changed = !animated && !sameJson(value, def.value(QStringLiteral("default")));
            break;
        }
        markChanged(inked, changed);
        editor->setEnabled(!animated);
        if (animated) r->setAnimated(true, QStringLiteral("Animated. Ask Kerf or edit the script to change it."));
    };
    binding.changed = [this, target, def] {
        for (const Clip &clip : m_clips)
            if (targets(clip, target) && !isAnimated(clip, target) && !sameJson(valueOf(clip, target, def), def.value(QStringLiteral("default"))))
                return true;
        return false;
    };
    binding.summary = [this, target, def, label] {
        for (const Clip &clip : m_clips) {
            if (!targets(clip, target) || isAnimated(clip, target)) continue;
            const QJsonValue value = valueOf(clip, target, def);
            if (sameJson(value, def.value(QStringLiteral("default")))) return QString();
            if (value.isBool()) return value.toBool() ? label : QString();
            const QString type = def.value(QStringLiteral("type")).toString();
            const QString text = type == QStringLiteral("enum") ? titleCase(value.toString())
                                 : type == QStringLiteral("file") && !QFileInfo(value.toString()).fileName().isEmpty() ? QFileInfo(value.toString()).fileName()
                                                                                                                         : value.toString();
            return def.value(QStringLiteral("type")).toString() == QStringLiteral("enum") && label == QStringLiteral("Blend") ? text
                                                                                                                             : QStringLiteral("%1 %2").arg(label, text);
        }
        return QString();
    };
    binding.resetOps = [this, target, def] { return resetOpsFor(target, def); };
    m_rows[rowIndex].values.append(binding);
    return editor;
}

void InspectorPanel::refreshValues() {
    for (const auto &refresher : std::as_const(m_refreshers)) refresher();
    for (const RowEntry &entry : std::as_const(m_rows)) {
        entry.row->setAnimated(false);
        bool changed = false;
        for (const Binding &binding : entry.values) {
            if (binding.refresh) binding.refresh();
            if (binding.changed) changed |= binding.changed();
        }
        entry.row->setChanged(changed);
    }
    for (const SectionEntry &entry : std::as_const(m_sections)) {
        QStringList pieces;
        bool changed = false;
        for (const int rowIndex : entry.rows) {
            for (const Binding &binding : m_rows.at(rowIndex).values) {
                if (binding.summary)
                    if (const QString piece = binding.summary(); !piece.isEmpty()) pieces << piece;
            }
            changed |= m_rows.at(rowIndex).row->isChanged();
        }
        if (entry.items) entry.items(entry.section);
        else entry.section->setSummary(entry.summary ? entry.summary() : pieces.join(QStringLiteral(" · ")));
        entry.section->setChanged(changed);
    }
}

// ----------------------------------------------------------------- kinds

void InspectorPanel::buildTransformRows(QVBoxLayout *into, int section, bool fitFill) {
    if (m_clips.isEmpty()) return;
    const Clip &first = m_clips.first();
    const QJsonObject scaleDef = defNamed(first, QStringLiteral("scale"));
    const QJsonObject xDef = defNamed(first, QStringLiteral("x"));
    const QJsonObject yDef = defNamed(first, QStringLiteral("y"));
    const QJsonObject rotationDef = defNamed(first, QStringLiteral("rotation"));
    if (!scaleDef.isEmpty()) {
        const int r = addRow(into, QStringLiteral("Scale"), section);
        ScrubField *scale = bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("scale")}, scaleDef);
        scale->setObjectName(QStringLiteral("prop_scale"));
        if (fitFill) {
            scale->setFixedWidth(kPairedField);
            // Two latching keys: the one that matches the clip is held in.
            auto *fit = new KeyBank({{QStringLiteral("Fit"), QStringLiteral("The whole picture in the frame"), QStringLiteral("fitKey")},
                                     {QStringLiteral("Fill"), QStringLiteral("The frame filled, edges cropped"), QStringLiteral("fillKey")}},
                                    true);
            fit->setObjectName(QStringLiteral("fitFill"));
            row(r)->controls()->addWidget(fit, 1);
            const auto fillScale = [this](const Clip &clip) {
                const double w = clip.source.value(QStringLiteral("width")).toDouble();
                const double h = clip.source.value(QStringLiteral("height")).toDouble();
                const QSize canvas = resolvedCanvasSize(m_project, m_project.value(QStringLiteral("sources")).toArray());
                if (w <= 0 || h <= 0 || canvas.isEmpty()) return 1.0;
                const double media = w / h;
                const double frame = double(canvas.width()) / canvas.height();
                return std::max(media / frame, frame / media);
            };
            // A choice shows only when every clip matches it.
            m_refreshers.append([this, fit, fillScale] {
                QSet<int> choices;
                for (const Clip &clip : std::as_const(m_clips)) {
                    const QJsonObject props = clip.clip.value(QStringLiteral("props")).toObject();
                    const double scaleValue = props.value(QStringLiteral("scale")).toDouble(1.0);
                    const bool centred = std::abs(props.value(QStringLiteral("x")).toDouble()) < 1e-6 && std::abs(props.value(QStringLiteral("y")).toDouble()) < 1e-6;
                    const double fill = fillScale(clip);
                    choices.insert(!centred ? -1 : std::abs(scaleValue - 1.0) < 1e-3 ? 0 : std::abs(scaleValue - fill) < 1e-3 * fill ? 1 : -1);
                }
                fit->setCurrent(choices.size() == 1 ? *choices.cbegin() : -1);
            });
            connect(fit, &KeyBank::activated, this, [this, fillScale](int index) {
                QJsonArray ops;
                for (const Clip &clip : std::as_const(m_clips)) {
                    const double fill = std::round(fillScale(clip) * 10000.0) / 10000.0;
                    QJsonObject props{{QStringLiteral("x"), QJsonValue()}, {QStringLiteral("y"), QJsonValue()},
                                      {QStringLiteral("scale"), index == 0 || std::abs(fill - 1.0) < 1e-6 ? QJsonValue() : QJsonValue(fill)}};
                    ops.append(QJsonObject{{QStringLiteral("op"), QStringLiteral("set")}, {QStringLiteral("clip"), clip.id}, {QStringLiteral("props"), props}});
                }
                emitOps(ops);
            });
        }
    }
    if (!xDef.isEmpty() && !yDef.isEmpty()) {
        const int r = addRow(into, QStringLiteral("Position"), section);
        bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("x")}, xDef)->setObjectName(QStringLiteral("prop_x"));
        bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("y")}, yDef, false, false)->setObjectName(QStringLiteral("prop_y"));
    }
    if (!rotationDef.isEmpty()) {
        const int r = addRow(into, QStringLiteral("Rotation"), section);
        ScrubField *rotation = bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("rotation")}, rotationDef);
        rotation->setObjectName(QStringLiteral("prop_rotation"));
        rotation->setFixedWidth(kSingleField);
        // The knob beside the field: the field follows it while it turns,
        // and its release (or a run of wheel steps) is one edit; its
        // double-click resets every clip, as the row's ↺ does.
        auto *knob = new RotationKnob;
        row(r)->controls()->addWidget(knob);
        row(r)->controls()->addStretch(1);
        m_refreshers.append([this, knob, rotationDef] {
            const Target target{Target::Kind::Prop, QStringLiteral("rotation")};
            knob->setDecimals(formatFor(m_clips.first(), rotationDef).decimals);
            knob->setAngle(valueOf(m_clips.first(), target, rotationDef).toDouble());
            knob->setEnabled(!isAnimated(m_clips.first(), target));
        });
        connect(knob, &RotationKnob::turned, rotation, &ScrubField::setValue);
        connect(knob, &RotationKnob::released, this, [this, rotationDef](double angle, double start) {
            commitNumber(Target{Target::Kind::Prop, QStringLiteral("rotation")}, rotationDef, angle, angle - start, m_clips.size() > 1);
        });
        connect(knob, &RotationKnob::resetRequested, this, [this, rotationDef] {
            emitOps(resetOpsFor(Target{Target::Kind::Prop, QStringLiteral("rotation")}, rotationDef));
        });
    }
}

void InspectorPanel::buildCompositingRows(QVBoxLayout *into, int section) {
    if (m_clips.isEmpty()) return;
    const QJsonObject opacityDef = defNamed(m_clips.first(), QStringLiteral("opacity"));
    const QJsonObject blendDef = defNamed(m_clips.first(), QStringLiteral("blend"));
    if (!opacityDef.isEmpty()) {
        const int r = addRow(into, QStringLiteral("Opacity"), section);
        bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("opacity")}, opacityDef, true)
            ->setObjectName(QStringLiteral("prop_opacity"));
    }
    if (!blendDef.isEmpty()) {
        const int r = addRow(into, QStringLiteral("Blend"), section);
        bindValue(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("blend")}, blendDef)->setObjectName(QStringLiteral("prop_blend"));
    }
}

void InspectorPanel::buildTimeSection(QVBoxLayout *into) {
    if (m_clips.isEmpty() || !std::all_of(m_clips.cbegin(), m_clips.cend(), [](const Clip &c) { return c.clip.contains(QStringLiteral("speed")); })) return;
    // Folded at first, heading the rack: its lamp and summary ("Speed
    // 200%") say when it changed, and Transform and Composite keep the room.
    const int section = addSection(into, QStringLiteral("time"), QStringLiteral("Time"));
    buildSpeedRow(sectionBody(section), section);
}

void InspectorPanel::buildSpeedRow(QVBoxLayout *into, int section) {
    const int r = addRow(into, QStringLiteral("Speed"), section);
    auto *field = newScrubField();
    field->setObjectName(QStringLiteral("speed"));
    field->setFixedWidth(kSingleField);
    NumberFormat format;
    format.kind = NumberFormat::Kind::Speed;
    format.unit = QStringLiteral("%");
    format.tightUnit = true;
    format.min = -2000.0;
    format.max = 2000.0;
    format.decimals = 1;
    format.step = 1.0;
    format.bigStep = 10.0;
    format.smallStep = 0.1;
    field->setFormat(format);
    field->setDefaultValue(100.0);
    row(r)->label()->setField(field);
    auto *reverse = new GlyphButton(GlyphButton::Glyph::Reverse);
    reverse->setObjectName(QStringLiteral("speedReverse"));
    reverse->setCheckable(true);
    reverse->setToolTip(QStringLiteral("Play backwards (\u2212100%)"));
    reverse->setAccessibleName(QStringLiteral("Play backwards"));
    // The presets are the field's own ⌄, a 16pt lane inside its right end;
    // Reverse follows the field.
    auto *presets = new GlyphButton(GlyphButton::Glyph::Menu, field);
    presets->setObjectName(QStringLiteral("speedPresets"));
    presets->setToolTip(QStringLiteral("Speed presets"));
    presets->setFixedSize(16, ScrubField::kHeight);
    presets->move(kSingleField - 18, 0);
    presets->setCursor(Qt::PointingHandCursor);
    field->setTrailingInset(14);
    auto *menu = new QMenu(presets);
    menu->setObjectName(QStringLiteral("speedPresetsMenu"));
    row(r)->controls()->addWidget(field);
    row(r)->controls()->addWidget(reverse);
    row(r)->controls()->addStretch(1);
    const auto display = [](const Clip &clip) {
        const double speed = clip.clip.value(QStringLiteral("speed")).toDouble(1.0) * 100.0;
        return clip.clip.value(QStringLiteral("reverse")).toBool() ? -speed : speed;
    };
    // A ratio for each clip from the display value (a minus sign reverses).
    const auto apply = [this](const std::function<double(const Clip &)> &next) {
        QJsonArray ops;
        for (const Clip &clip : std::as_const(m_clips)) {
            const double current = clip.clip.value(QStringLiteral("speed")).toDouble(1.0) * (clip.clip.value(QStringLiteral("reverse")).toBool() ? -1.0 : 1.0);
            double ratio = next(clip) / 100.0;
            if (std::abs(ratio) < 0.01) ratio = ratio < 0 ? -0.01 : 0.01;
            if (std::abs(ratio - current) < 1e-6) continue;
            ops.append(QJsonObject{{QStringLiteral("op"), QStringLiteral("speed")}, {QStringLiteral("clip"), clip.id},
                                   {QStringLiteral("speed"), (ratio < 0 ? QStringLiteral("-") : QString()) + ratioText(ratio)}});
        }
        emitOps(ops);
    };
    for (const int percent : {25, 50, 100, 150, 200, 400}) {
        QAction *action = menu->addAction(QStringLiteral("%1%").arg(percent));
        action->setObjectName(QStringLiteral("speed%1").arg(percent));
        connect(action, &QAction::triggered, this, [apply, display, percent] {
            apply([display, percent](const Clip &clip) { return display(clip) < 0 ? -percent : percent; });
        });
    }
    connect(presets, &QAbstractButton::clicked, this, [presets, menu] { menu->popup(presets->mapToGlobal(QPoint(0, presets->height()))); });
    connect(reverse, &QAbstractButton::clicked, this, [apply, display] { apply([display](const Clip &clip) { return -display(clip); }); });
    connect(field, &ScrubField::committed, this, [this, apply, display](double value, double delta, bool relative) {
        const bool several = m_clips.size() > 1;
        apply([value, delta, relative, several, display](const Clip &clip) { return relative && several ? display(clip) + delta : value; });
    });
    Binding binding;
    binding.refresh = [this, field, reverse, display] {
        QSet<QString> shown;
        for (const Clip &clip : std::as_const(m_clips)) shown.insert(formatNumber(field->format(), display(clip)));
        field->setValue(display(m_clips.first()));
        field->setMixed(shown.size() > 1);
        const QSignalBlocker blocker(reverse);
        reverse->setChecked(display(m_clips.first()) < 0);
    };
    binding.changed = [this, display] {
        return std::any_of(m_clips.cbegin(), m_clips.cend(), [display](const Clip &clip) { return std::abs(display(clip) - 100.0) > 1e-6; });
    };
    binding.summary = [this, field, display] {
        const double value = display(m_clips.first());
        return std::abs(value - 100.0) > 1e-6 ? QStringLiteral("Speed ") + formatWithUnit(field->format(), value) : QString();
    };
    binding.resetOps = [this] {
        QJsonArray ops;
        for (const Clip &clip : std::as_const(m_clips))
            if (clip.clip.value(QStringLiteral("speedRatio")).toString(QStringLiteral("1")) != QStringLiteral("1"))
                ops.append(QJsonObject{{QStringLiteral("op"), QStringLiteral("speed")}, {QStringLiteral("clip"), clip.id}, {QStringLiteral("speed"), QStringLiteral("1")}});
        return ops;
    };
    m_rows[r].values.append(binding);
}

void InspectorPanel::buildCropSection(QVBoxLayout *into) {
    const Clip &first = m_clips.first();
    const QJsonObject l = defNamed(first, QStringLiteral("crop_l")), rr = defNamed(first, QStringLiteral("crop_r"));
    const QJsonObject t = defNamed(first, QStringLiteral("crop_t")), b = defNamed(first, QStringLiteral("crop_b"));
    if (l.isEmpty() || rr.isEmpty() || t.isEmpty() || b.isEmpty()) return;
    const int section = addSection(into, QStringLiteral("crop"), QStringLiteral("Crop"));
    const int across = addRow(sectionBody(section), QStringLiteral("Left · Right"), section);
    const auto field = [this](int r, const char *name, const QJsonObject &def, const char *prefix, bool primary) {
        ScrubField *f = bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, QString::fromLatin1(name)}, def, false, primary);
        f->setObjectName(QStringLiteral("prop_") + QString::fromLatin1(name));
        f->setProperty("prefix", QString::fromLatin1(prefix));
    };
    field(across, "crop_l", l, "L", true);
    field(across, "crop_r", rr, "R", false);
    const int down = addRow(sectionBody(section), QStringLiteral("Top · Bottom"), section);
    field(down, "crop_t", t, "T", true);
    field(down, "crop_b", b, "B", false);
    // "Bottom 60 px" for one side, "L 10 · B 60 px" for several.
    m_sections[section].summary = [this, l, rr, t, b] {
        const Clip &clip = m_clips.first();
        QStringList sides;
        QString single;
        const struct {
            const QJsonObject *def;
            const char *letter;
            const char *word;
        } all[] = {{&l, "L", "Left"}, {&rr, "R", "Right"}, {&t, "T", "Top"}, {&b, "B", "Bottom"}};
        for (const auto &side : all) {
            const Target target{Target::Kind::Prop, side.def->value(QStringLiteral("name")).toString()};
            const NumberFormat format = formatFor(clip, *side.def);
            const double value = displayOf(clip, target, *side.def);
            if (formatNumber(format, value) == formatNumber(format, 0.0)) continue;
            sides << QStringLiteral("%1 %2").arg(QString::fromLatin1(side.letter), formatNumber(format, value));
            single = QStringLiteral("%1 %2").arg(QString::fromLatin1(side.word), formatWithUnit(format, value));
        }
        if (sides.isEmpty()) return QString();
        return sides.size() == 1 ? single : sides.join(QStringLiteral(" · ")) + QStringLiteral(" px");
    };
}

void InspectorPanel::buildAnchorSection(QVBoxLayout *into) {
    const Clip &first = m_clips.first();
    const QJsonObject ax = defNamed(first, QStringLiteral("anchor_x")), ay = defNamed(first, QStringLiteral("anchor_y"));
    const QJsonObject sx = defNamed(first, QStringLiteral("scale_x")), sy = defNamed(first, QStringLiteral("scale_y"));
    if (ax.isEmpty() && sx.isEmpty()) return;
    const int section = addSection(into, QStringLiteral("anchor"), QStringLiteral("Anchor & flip"));
    if (!ax.isEmpty() && !ay.isEmpty()) {
        const int r = addRow(sectionBody(section), QStringLiteral("Anchor"), section);
        bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("anchor_x")}, ax)->setObjectName(QStringLiteral("prop_anchor_x"));
        bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("anchor_y")}, ay, false, false)->setObjectName(QStringLiteral("prop_anchor_y"));
    }
    if (sx.isEmpty() || sy.isEmpty()) return;
    // Flip: the sign of each axis's stretch, a latching key each; Stretch: its size.
    const int flipRow = addRow(sectionBody(section), QStringLiteral("Flip"), section);
    const int stretchRow = addRow(sectionBody(section), QStringLiteral("Stretch"), section);
    auto *flips = new KeyBank({{QStringLiteral("H"), QStringLiteral("Flip horizontally"), QStringLiteral("flip_H")},
                               {QStringLiteral("V"), QStringLiteral("Flip vertically"), QStringLiteral("flip_V")}},
                              false);
    flips->setObjectName(QStringLiteral("flip"));
    flips->setFixedWidth(kSingleField);
    row(flipRow)->controls()->addWidget(flips);
    int keyIndex = 0;
    for (const auto &[name, def, letter] : {std::tuple{QStringLiteral("scale_x"), sx, QStringLiteral("H")},
                                            std::tuple{QStringLiteral("scale_y"), sy, QStringLiteral("V")}}) {
        const Target target{Target::Kind::Prop, name};
        const int index = keyIndex++;
        connect(flips, &KeyBank::activated, this, [this, index, target, def = def](int pressed) {
            if (pressed != index) return;
            QJsonArray ops;
            for (const Clip &clip : std::as_const(m_clips)) {
                if (isAnimated(clip, target)) continue;
                const double flipped = -valueOf(clip, target, def).toDouble();
                ops.append(opFor(clip, target, std::abs(flipped - 1.0) < 1e-9 ? QJsonValue() : QJsonValue(flipped)));
            }
            emitOps(ops);
        });
        Binding flip;
        flip.refresh = [this, flips, index, target, def = def] {
            flips->setLatched(index, valueOf(m_clips.first(), target, def).toDouble() < 0);
        };
        flip.changed = [this, target, def = def] { return valueOf(m_clips.first(), target, def).toDouble() < 0; };
        flip.summary = [this, target, def = def, letter = letter] {
            return valueOf(m_clips.first(), target, def).toDouble() < 0 ? QStringLiteral("Flipped ") + letter : QString();
        };
        flip.resetOps = [this, target, def = def] {
            QJsonArray ops;
            for (const Clip &clip : std::as_const(m_clips)) {
                const double value = valueOf(clip, target, def).toDouble();
                if (value < 0 && !isAnimated(clip, target))
                    ops.append(opFor(clip, target, std::abs(-value - 1.0) < 1e-9 ? QJsonValue() : QJsonValue(-value)));
            }
            return ops;
        };
        m_rows[flipRow].values.append(flip);
        // Stretch shows the size; edits keep the sign.
        auto *field = newScrubField();
        field->setObjectName(QStringLiteral("stretch_") + letter);
        row(stretchRow)->controls()->addWidget(field, 1);
        if (letter == QStringLiteral("H")) row(stretchRow)->label()->setField(field);
        Binding stretch;
        stretch.refresh = [this, field, target, def = def, letter = letter] {
            NumberFormat format = formatFor(m_clips.first(), def);
            // X and Y, as Position's fields: "H" is already Flip's
            // horizontal in the row above, and cannot mean height here.
            format.prefix = letter == QStringLiteral("H") ? QStringLiteral("X") : QStringLiteral("Y");
            format.min = 0.0;
            field->setFormat(format);
            field->setDefaultValue(100.0);
            field->setValue(std::abs(valueOf(m_clips.first(), target, def).toDouble()) * 100.0);
        };
        stretch.changed = [this, target, def = def] { return std::abs(std::abs(valueOf(m_clips.first(), target, def).toDouble()) - 1.0) > 5e-4; };
        stretch.summary = [this, field, target, def = def] {
            const double size = std::abs(valueOf(m_clips.first(), target, def).toDouble()) * 100.0;
            return std::abs(size - 100.0) > 0.05 ? QStringLiteral("%1 %2").arg(field->format().prefix, formatWithUnit(field->format(), size)) : QString();
        };
        stretch.resetOps = [this, target, def = def] {
            QJsonArray ops;
            for (const Clip &clip : std::as_const(m_clips)) {
                const double value = valueOf(clip, target, def).toDouble();
                if (std::abs(std::abs(value) - 1.0) > 5e-4 && !isAnimated(clip, target))
                    ops.append(opFor(clip, target, value < 0 ? QJsonValue(-1.0) : QJsonValue()));
            }
            return ops;
        };
        m_rows[stretchRow].values.append(stretch);
        connect(field, &ScrubField::committed, this, [this, target, def = def](double value, double delta, bool relative) {
            QJsonArray ops;
            for (const Clip &clip : std::as_const(m_clips)) {
                if (isAnimated(clip, target)) continue;
                const double current = valueOf(clip, target, def).toDouble();
                const double size = std::max(0.0, (relative && m_clips.size() > 1 ? std::abs(current) * 100.0 + delta : value) / 100.0);
                const double next = current < 0 ? -size : size;
                ops.append(opFor(clip, target, std::abs(next - 1.0) < 1e-9 ? QJsonValue() : QJsonValue(next)));
            }
            emitOps(ops);
        });
    }
    row(flipRow)->controls()->addStretch(1);
}

void InspectorPanel::buildLinkedAudioSection(QVBoxLayout *into) {
    if (m_clips.size() != 1 || m_clips.first().linked.isEmpty()) return;
    const Clip &first = m_clips.first();
    const QJsonArray defs = m_project.value(QStringLiteral("properties")).toObject().value(first.linkedTrack.value(QStringLiteral("type")).toString()).toArray();
    QJsonObject gain, pan;
    for (const QJsonValue &def : defs) {
        if (def.toObject().value(QStringLiteral("name")).toString() == QStringLiteral("gain")) gain = def.toObject();
        if (def.toObject().value(QStringLiteral("name")).toString() == QStringLiteral("pan")) pan = def.toObject();
    }
    if (gain.isEmpty() && pan.isEmpty()) return;
    const int section = addSection(into, QStringLiteral("linkedAudio"),
                                   QStringLiteral("Audio · ") + trackBadge(m_project, first.linkedTrack.value(QStringLiteral("id")).toString()).badge);
    if (!gain.isEmpty()) {
        const int r = addRow(sectionBody(section), QStringLiteral("Volume"), section);
        bindNumber(r, row(r)->controls(), Target{Target::Kind::Linked, QStringLiteral("gain")}, gain, true)->setObjectName(QStringLiteral("linked_gain"));
    }
    if (!pan.isEmpty()) {
        const int r = addRow(sectionBody(section), QStringLiteral("Pan"), section);
        bindNumber(r, row(r)->controls(), Target{Target::Kind::Linked, QStringLiteral("pan")}, pan, true)->setObjectName(QStringLiteral("linked_pan"));
    }
}

void InspectorPanel::buildSourceSection(QVBoxLayout *into) {
    if (m_clips.size() != 1 || m_clips.first().source.isEmpty()) return;
    const QJsonObject source = m_clips.first().source;
    const int section = addSection(into, QStringLiteral("source"), QStringLiteral("Source"));
    QVBoxLayout *body = sectionBody(section);
    const double width = source.value(QStringLiteral("width")).toDouble();
    const double height = source.value(QStringLiteral("height")).toDouble();
    const double fps = source.value(QStringLiteral("fps")).toDouble(m_project.value(QStringLiteral("fps")).toDouble(24.0));
    QString format = width > 0 ? QStringLiteral("%1 × %2 · %3 fps").arg(width).arg(height).arg(QString::number(fps, 'g', 4)) : QString();
    const auto text = [this, body, section](const QString &label, const QString &value, const QString &name) {
        const int r = addRow(body, label, section);
        auto *plain = new QLabel(value);
        plain->setObjectName(name);
        plain->setFont(sizedFont(plain->font(), 12));
        plain->setToolTip(value);
        plain->setMinimumWidth(0);
        plain->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
        row(r)->controls()->addWidget(plain, 1);
    };
    text(QStringLiteral("File"), sourceFileName(source), QStringLiteral("sourceFile"));
    if (!format.isEmpty()) text(QStringLiteral("Format"), format, QStringLiteral("sourceFormat"));
    if (source.value(QStringLiteral("channels")).toInt() > 0)
        text(QStringLiteral("Audio"), QStringLiteral("%1 ch · %2 kHz").arg(source.value(QStringLiteral("channels")).toInt())
                                          .arg(QString::number(source.value(QStringLiteral("sampleRate")).toDouble() / 1000.0, 'g', 4)),
             QStringLiteral("sourceAudio"));
    // Source In and Out on the media's own rate; both slip the clip.
    if (m_clips.first().clip.contains(QStringLiteral("in"))) {
        const int r = addRow(body, QStringLiteral("In · Out"), section);
        NumberFormat tc;
        tc.kind = NumberFormat::Kind::Timecode;
        tc.fps = fps;
        tc.min = 0.0;
        tc.max = 1e9;
        tc.step = 1.0;
        tc.bigStep = 10.0;
        tc.smallStep = 1.0;
        for (const char *edge : {"in", "out"}) {
            auto *field = newScrubField();
            field->setObjectName(QStringLiteral("source_%1").arg(QString::fromLatin1(edge)));
            field->setFormat(tc);
            row(r)->controls()->addWidget(field, 1);
            if (QString::fromLatin1(edge) == QStringLiteral("in")) row(r)->label()->setField(field);
            const QString name = QString::fromLatin1(edge);
            m_refreshers.append([this, field, name, fps] {
                field->setValue(std::round(m_clips.first().clip.value(name).toDouble() * fps));
            });
            connect(field, &ScrubField::committed, this, [this, field, name, fps](double value, double, bool) {
                const Clip &clip = m_clips.first();
                const double speed = std::max(0.01, clip.clip.value(QStringLiteral("speed")).toDouble(1.0));
                const double seconds = value / fps - clip.clip.value(name).toDouble();
                const qint64 frames = std::llround(seconds / speed * m_project.value(QStringLiteral("fps")).toDouble(24.0));
                Q_UNUSED(field)
                if (frames == 0) return;
                emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("slip")}, {QStringLiteral("clip"), clip.id},
                                               {QStringLiteral("frames"), frames}}});
            });
        }
    }
    auto *reveal = new QPushButton(QStringLiteral("Reveal in Project"));
    reveal->setObjectName(QStringLiteral("revealSource"));
    reveal->setFlat(true);
    reveal->setProperty("inspectorLink", true);
    connect(reveal, &QPushButton::clicked, this, [this] { emit revealRequested(m_clips.first().clip.value(QStringLiteral("sourceId")).toString()); });
    auto *revealRow = new QHBoxLayout;
    revealRow->setContentsMargins(InspectorRow::kLabelWidth + 8, 4, 0, 0);
    revealRow->addWidget(reveal);
    revealRow->addStretch(1);
    body->addLayout(revealRow);
    m_sections[section].summary = [format] { return format; };
}

void InspectorPanel::buildVideo(QVBoxLayout *into, bool generator) {
    const bool fit = !generator;
    // What one changes first, open at the top (Transform, Composite), then
    // the rack of folded ones, Time first.
    const int transform = addSection(into, QStringLiteral("transform"), QStringLiteral("Transform"), true);
    buildTransformRows(sectionBody(transform), transform, fit);
    const int compositing = addSection(into, QStringLiteral("compositing"), QStringLiteral("Composite"), true);
    buildCompositingRows(sectionBody(compositing), compositing);
    buildTimeSection(into);
    buildCropSection(into);
    buildAnchorSection(into);
    buildLinkedAudioSection(into);
    buildMasksSection(into);
    buildEffectsSection(into, false);
    buildSourceSection(into);
}

void InspectorPanel::buildAudio(QVBoxLayout *into) {
    const Clip &first = m_clips.first();
    const QJsonObject gain = defNamed(first, QStringLiteral("gain"));
    const QJsonObject pan = defNamed(first, QStringLiteral("pan"));
    // The level and the rest of the sound clip's own properties, open.
    const int mix = addSection(into, QStringLiteral("mix"), QStringLiteral("Mix"), true);
    QVBoxLayout *body = sectionBody(mix);
    if (!gain.isEmpty()) {
        const int r = addRow(body, QStringLiteral("Volume"), mix);
        bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("gain")}, gain, true)->setObjectName(QStringLiteral("prop_gain"));
    }
    if (!pan.isEmpty()) {
        const int r = addRow(body, QStringLiteral("Pan"), mix);
        bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("pan")}, pan, true)->setObjectName(QStringLiteral("prop_pan"));
    }
    // Anything else the catalog gives a sound clip.
    for (const QJsonValue &value : defsOf(first)) {
        const QJsonObject def = value.toObject();
        const QString name = def.value(QStringLiteral("name")).toString();
        if (name == QStringLiteral("gain") || name == QStringLiteral("pan")) continue;
        const int r = addRow(body, presentationLabel(def), mix);
        if (def.value(QStringLiteral("type")).toString() == QStringLiteral("number"))
            bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, name}, def, def.contains(QStringLiteral("max")))->setObjectName(QStringLiteral("prop_") + name);
        else
            bindValue(r, row(r)->controls(), Target{Target::Kind::Prop, name}, def)->setObjectName(QStringLiteral("prop_") + name);
    }
    buildTimeSection(into);
    buildEffectsSection(into, false);
    buildSourceSection(into);
}

void InspectorPanel::buildTitle(QVBoxLayout *into) {
    const Clip &first = m_clips.first();
    const QJsonObject text = defNamed(first, QStringLiteral("text"));
    const QJsonObject font = defNamed(first, QStringLiteral("font"));
    const QJsonObject size = defNamed(first, QStringLiteral("size"));
    const QJsonObject color = defNamed(first, QStringLiteral("color"));
    const QJsonObject align = defNamed(first, QStringLiteral("align"));
    const QJsonObject y = defNamed(first, QStringLiteral("y"));
    // The words and how they are set, open at the top.
    const int words = addSection(into, QStringLiteral("text"), QStringLiteral("Text"), true);
    QVBoxLayout *body = sectionBody(words);
    if (!text.isEmpty()) {
        const int r = addRow(body, QStringLiteral("Text"), words);
        InspectorRow *textRow = row(r);
        textRow->setMinimumHeight(InspectorRow::kHeight);
        textRow->setMaximumHeight(QWIDGETSIZE_MAX);
        textRow->label()->setAlignment(Qt::AlignLeft | Qt::AlignTop);
        textRow->label()->setContentsMargins(0, 8, 0, 0);
        auto *edit = new QPlainTextEdit;
        edit->setObjectName(QStringLiteral("titleText"));
        edit->setProperty("inspector", true);
        edit->setTabChangesFocus(true);
        edit->setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
        const int line = QFontMetrics(edit->font()).lineSpacing();
        edit->setFixedHeight(line * 3 + 12);
        textRow->controls()->addWidget(edit, 1);
        textRow->layout()->setContentsMargins(0, 4, 0, 4);
        const Target target{Target::Kind::Prop, QStringLiteral("text")};
        edit->installEventFilter(this);
        edit->setProperty("commit", true);
        connect(edit->document(), &QTextDocument::contentsChanged, edit, [edit, line] {
            const int lines = std::clamp(int(edit->document()->size().height()), 3, 6);
            edit->setFixedHeight(line * lines + 12);
        });
        Binding binding;
        binding.refresh = [this, edit, target, text] {
            if (edit->hasFocus()) return;
            const QSignalBlocker blocker(edit);
            const QString value = valueOf(m_clips.first(), target, text).toString();
            if (edit->toPlainText() != value) edit->setPlainText(value);
        };
        binding.changed = [] { return false; };
        m_rows[r].values.append(binding);
    }
    if (!font.isEmpty() || !size.isEmpty()) {
        const int r = addRow(body, QStringLiteral("Font"), words);
        if (!font.isEmpty()) {
            auto *combo = new FontPopup;
            combo->setObjectName(QStringLiteral("titleFont"));
            combo->setProperty("inspector", true);
            combo->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
            combo->setMinimumContentsLength(4);
            combo->setMinimumWidth(64);
            combo->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
            // The generic families by the names they resolve to; the model
            // keeps the generic name.
            for (const QString &family : {QStringLiteral("sans-serif"), QStringLiteral("serif"), QStringLiteral("monospace")}) {
                combo->addItem(resolvedFamily(family), family);
                combo->setItemData(combo->count() - 1, family, Qt::ToolTipRole);
            }
            combo->addItem(QStringLiteral("Other…"), QString());
            row(r)->controls()->addWidget(combo, 1);
            const Target target{Target::Kind::Prop, QStringLiteral("font")};
            connect(combo, &QComboBox::activated, this, [this, combo, target, font](int index) {
                QString family = combo->itemData(index).toString();
                if (family.isEmpty()) family = combo->itemText(index);
                if (combo->itemData(index).toString().isEmpty() && family == QStringLiteral("Other…")) {
                    bool ok = false;
                    family = QInputDialog::getText(this, QStringLiteral("Font"), QStringLiteral("Family"), QLineEdit::Normal,
                                                   valueOf(m_clips.first(), target, font).toString(), &ok);
                    if (!ok || family.trimmed().isEmpty()) {
                        refreshValues();
                        return;
                    }
                }
                commitValue(target, font, family.trimmed());
            });
            Binding binding;
            binding.refresh = [this, combo, target, font] {
                const QString family = valueOf(m_clips.first(), target, font).toString();
                const QSignalBlocker blocker(combo);
                int index = combo->findData(family);
                if (index < 0) index = combo->findText(family);
                if (index < 0) {
                    combo->insertItem(0, family, family);
                    index = 0;
                }
                combo->setCurrentIndex(index);
                markChanged(combo, !sameJson(valueOf(m_clips.first(), target, font), font.value(QStringLiteral("default"))));
            };
            binding.changed = [this, target, font] { return !sameJson(valueOf(m_clips.first(), target, font), font.value(QStringLiteral("default"))); };
            binding.resetOps = [this, target, font] { return resetOpsFor(target, font); };
            m_rows[r].values.append(binding);
        }
        if (!size.isEmpty()) {
            // On a row of its own: the family needs the pop-up's whole width.
            const int sizeRow = font.isEmpty() ? r : addRow(body, QStringLiteral("Size"), words);
            ScrubField *field = bindNumber(sizeRow, row(sizeRow)->controls(), Target{Target::Kind::Prop, QStringLiteral("size")}, size);
            field->setObjectName(QStringLiteral("titleSize"));
            field->setFixedWidth(kPairedField);
            row(sizeRow)->controls()->addStretch(1);
        }
    }
    if (!color.isEmpty()) {
        const int r = addRow(body, QStringLiteral("Colour"), words);
        bindValue(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("color")}, color)->setObjectName(QStringLiteral("titleColor"));
    }
    if (!align.isEmpty()) {
        const int r = addRow(body, QStringLiteral("Align"), words);
        auto *field = new SegmentedField({{QString(), SegmentedField::Glyph::AlignLeft, QStringLiteral("Left")},
                                          {QString(), SegmentedField::Glyph::AlignCenter, QStringLiteral("Centre")},
                                          {QString(), SegmentedField::Glyph::AlignRight, QStringLiteral("Right")}});
        field->setObjectName(QStringLiteral("titleAlign"));
        field->setFixedWidth(108);
        row(r)->controls()->addWidget(field);
        row(r)->controls()->addStretch(1);
        const QStringList values{QStringLiteral("left"), QStringLiteral("center"), QStringLiteral("right")};
        field->setDefaultIndex(int(values.indexOf(align.value(QStringLiteral("default")).toString())));
        const Target target{Target::Kind::Prop, QStringLiteral("align")};
        connect(field, &SegmentedField::activated, this, [this, values, target, align](int index) { commitValue(target, align, values.value(index)); });
        Binding binding;
        binding.refresh = [this, field, values, target, align] { field->setCurrent(int(values.indexOf(valueOf(m_clips.first(), target, align).toString()))); };
        binding.changed = [this, target, align] { return !sameJson(valueOf(m_clips.first(), target, align), align.value(QStringLiteral("default"))); };
        binding.resetOps = [this, target, align] { return resetOpsFor(target, align); };
        m_rows[r].values.append(binding);
    }
    if (!y.isEmpty()) {
        // Where the line sits: y of -0.30, 0, or +0.28 frame heights.
        const int r = addRow(body, QStringLiteral("Place"), words);
        auto *field = new SegmentedField({{QStringLiteral("Top"), SegmentedField::Glyph::None, QStringLiteral("At the top")},
                                          {QStringLiteral("Middle"), SegmentedField::Glyph::None, QStringLiteral("In the middle")},
                                          {QStringLiteral("Lower"), SegmentedField::Glyph::None, QStringLiteral("On the lower third")}});
        field->setObjectName(QStringLiteral("titlePlace"));
        row(r)->controls()->addWidget(field, 1);
        const QList<double> places{-0.30, 0.0, 0.28};
        for (int index = 0; index < places.size(); ++index)
            if (std::abs(places.at(index) - y.value(QStringLiteral("default")).toDouble()) < 1e-3) field->setDefaultIndex(index);
        const Target target{Target::Kind::Prop, QStringLiteral("y")};
        connect(field, &SegmentedField::activated, this, [this, places, target, y](int index) {
            commitValue(target, y, std::abs(places.value(index)) < 1e-9 ? QJsonValue(0.0) : QJsonValue(places.value(index)));
        });
        m_refreshers.append([this, field, places, target, y] {
            const double value = valueOf(m_clips.first(), target, y).toDouble();
            int chosen = -1;
            for (int index = 0; index < places.size(); ++index)
                if (std::abs(places.at(index) - value) < 1e-3) chosen = index;
            field->setCurrent(chosen);
        });
    }
    const int transform = addSection(into, QStringLiteral("transform"), QStringLiteral("Transform"));
    buildTransformRows(sectionBody(transform), transform, false);
    const int compositing = addSection(into, QStringLiteral("compositing"), QStringLiteral("Composite"));
    buildCompositingRows(sectionBody(compositing), compositing);
    buildCropSection(into);
    buildAnchorSection(into);
    if (m_maskable) buildMasksSection(into);
    buildEffectsSection(into, false);
}

void InspectorPanel::buildSolid(QVBoxLayout *into) {
    const QJsonObject color = defNamed(m_clips.first(), QStringLiteral("color"));
    // Its colour with how it is laid over what is under it, open.
    const int compositing = addSection(into, QStringLiteral("compositing"), QStringLiteral("Composite"), true);
    if (!color.isEmpty()) {
        const int r = addRow(sectionBody(compositing), QStringLiteral("Colour"), compositing);
        bindValue(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("color")}, color)->setObjectName(QStringLiteral("solidColor"));
    }
    buildCompositingRows(sectionBody(compositing), compositing);
    const int transform = addSection(into, QStringLiteral("transform"), QStringLiteral("Transform"));
    buildTransformRows(sectionBody(transform), transform, false);
    buildCropSection(into);
    buildAnchorSection(into);
    buildMasksSection(into);
    buildEffectsSection(into, false);
}

void InspectorPanel::buildAdjustment(QVBoxLayout *into) {
    const int compositing = addSection(into, QStringLiteral("compositing"), QStringLiteral("Composite"), true);
    buildCompositingRows(sectionBody(compositing), compositing);
    buildEffectsSection(into, true);
    const int transform = addSection(into, QStringLiteral("transform"), QStringLiteral("Transform"));
    buildTransformRows(sectionBody(transform), transform, false);
    buildCropSection(into);
    buildMasksSection(into);
}

void InspectorPanel::buildCaption(QVBoxLayout *into) {
    // Captions carry no properties; the text is read-only until the editor
    // can set it.
    const int words = addSection(into, QStringLiteral("caption"), QStringLiteral("Caption"), true);
    const int r = addRow(sectionBody(words), QStringLiteral("Text"), words);
    InspectorRow *textRow = row(r);
    textRow->setMaximumHeight(QWIDGETSIZE_MAX);
    textRow->label()->setAlignment(Qt::AlignLeft | Qt::AlignTop);
    textRow->label()->setContentsMargins(0, 8, 0, 0);
    textRow->layout()->setContentsMargins(0, 0, 0, 4);
    auto *text = new QLabel;
    text->setObjectName(QStringLiteral("captionText"));
    text->setWordWrap(true);
    text->setFont(sizedFont(text->font(), 12));
    text->setContentsMargins(0, 7, 0, 0);
    text->setTextInteractionFlags(Qt::TextSelectableByMouse);
    auto *column = new QVBoxLayout;
    column->setSpacing(8);
    column->addWidget(text);
    // Until captions take typed text: a link that asks Kerf, on its own line
    // 8 points under the text, 12 points in the secondary ink after a
    // speech bubble (as Add effect has its +), underlined under the
    // pointer, so it never reads as the caption's next line.
    auto *hint = new QPushButton(QStringLiteral("Ask Kerf to change it"));
    hint->setToolTip(QStringLiteral("Ask Kerf to change the caption's text"));
    hint->setObjectName(QStringLiteral("captionHint"));
    hint->setFlat(true);
    hint->setProperty("inspectorLink", true);
    hint->setProperty("quiet", true);
    hint->setProperty("underlineOnHover", true);
    hint->setIcon(glyphIcon(QSize(12, 12), [](QPainter &p, const QColor &) {
        drawInspectorGlyph(p, GlyphButton::Glyph::Bubble, QRectF(0, 0, 12, 12), editorTheme().textSecondary);
    }));
    hint->setIconSize(QSize(12, 12));
    hint->setCursor(Qt::PointingHandCursor);
    connect(hint, &QPushButton::clicked, this, [this] { askAbout(QLatin1Char('@') + displayName(m_clips.first()) + QStringLiteral(" caption text:")); });
    column->addWidget(hint, 0, Qt::AlignLeft);
    textRow->controls()->addLayout(column, 1);
    m_refreshers.append([this, text] { text->setText(m_clips.first().clip.value(QStringLiteral("text")).toString()); });
}

void InspectorPanel::buildGenerator(QVBoxLayout *into) {
    // A generator the table does not know: its own parameters first.
    const QJsonArray params = m_project.value(QStringLiteral("generators")).toObject().value(m_clips.first().clip.value(QStringLiteral("fx")).toString()).toArray();
    const int own = params.isEmpty() ? -1 : addSection(into, QStringLiteral("generator"), kindName(m_clips.first()), true);
    for (const QJsonValue &value : params) {
        const QJsonObject def = value.toObject();
        const QString name = def.value(QStringLiteral("name")).toString();
        const int r = addRow(sectionBody(own), presentationLabel(def), own);
        if (def.value(QStringLiteral("type")).toString() == QStringLiteral("number"))
            bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, name}, def, def.contains(QStringLiteral("max")))->setObjectName(QStringLiteral("prop_") + name);
        else
            bindValue(r, row(r)->controls(), Target{Target::Kind::Prop, name}, def)->setObjectName(QStringLiteral("prop_") + name);
    }
    const int transform = addSection(into, QStringLiteral("transform"), QStringLiteral("Transform"));
    buildTransformRows(sectionBody(transform), transform, false);
    const int compositing = addSection(into, QStringLiteral("compositing"), QStringLiteral("Composite"));
    buildCompositingRows(sectionBody(compositing), compositing);
    buildCropSection(into);
    buildAnchorSection(into);
    if (m_maskable) buildMasksSection(into);
    buildEffectsSection(into, false);
}

void InspectorPanel::buildSequence(QVBoxLayout *into) {
    const EditorTheme &theme = editorTheme();
    const double fps = m_project.value(QStringLiteral("fps")).toDouble(24.0);
    const bool drop = m_project.value(QStringLiteral("dropFrame")).toBool();
    const auto plain = [](const QString &value, const QString &name) {
        auto *label = new QLabel(value);
        label->setObjectName(name);
        label->setFont(sizedFont(label->font(), 12));
        label->setMinimumWidth(0);
        label->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
        return label;
    };
    // Format: the frame size is the one thing that can change here.
    const int format = addSection(into, QStringLiteral("format"), QStringLiteral("Format"), true);
    {
        const int r = addRow(sectionBody(format), QStringLiteral("Frame size"), format);
        auto *size = popup(QStringLiteral("frameSize"));
        const QSize canvas = resolvedCanvasSize(m_project, m_project.value(QStringLiteral("sources")).toArray());
        int current = -1;
        for (const CanvasPreset &preset : canvasPresets()) {
            // Named as the toolbar names it ("9:16 · 1080 × 1920"), with thin
            // spaces round the × so the whole value fits the pop-up at 300.
            const QSize presetSize(preset.width, preset.height);
            size->addItem(canvasSizeLabel(presetSize), QStringLiteral("%1x%2").arg(preset.width).arg(preset.height));
            if (canvas == presetSize) current = size->count() - 1;
        }
        if (current < 0) {
            size->addItem(canvasSizeLabel(canvas), QStringLiteral("%1x%2").arg(canvas.width()).arg(canvas.height()));
            current = size->count() - 1;
        }
        size->addItem(QStringLiteral("Custom…"), QString());
        size->setCurrentIndex(current);
        row(r)->controls()->addWidget(size, 1);
        connect(size, &QComboBox::activated, this, [this, size, current](int index) {
            QString value = size->itemData(index).toString();
            if (value.isEmpty()) {
                bool ok = false;
                value = QInputDialog::getText(this, QStringLiteral("Frame Size"), QStringLiteral("Width × height"), QLineEdit::Normal,
                                              size->itemData(current).toString(), &ok);
                if (!ok) {
                    size->setCurrentIndex(current);
                    return;
                }
            }
            static const QRegularExpression pattern(QStringLiteral("^\\s*(\\d+)\\s*[x×*]\\s*(\\d+)\\s*$"));
            const QRegularExpressionMatch match = pattern.match(value);
            if (!match.hasMatch() || match.captured(1).toInt() < 2 || match.captured(2).toInt() < 2) {
                size->setCurrentIndex(current);
                return;
            }
            emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("set-sequence")},
                                           {QStringLiteral("width"), match.captured(1).toInt()},
                                           {QStringLiteral("height"), match.captured(2).toInt()}}});
        });
        const auto line = [&](const QString &label, const QString &value, const QString &name) {
            const int rr = addRow(sectionBody(format), label, format);
            row(rr)->controls()->addWidget(plain(value, name), 1);
        };
        line(QStringLiteral("Frame rate"), QStringLiteral("%1 fps%2").arg(QString::number(fps, 'g', 4), drop ? QStringLiteral(" · drop-frame") : QString()),
             QStringLiteral("sequenceRate"));
        if (const int rate = m_project.value(QStringLiteral("sampleRate")).toInt(); rate > 0)
            line(QStringLiteral("Audio rate"), QStringLiteral("%1 kHz").arg(QString::number(rate / 1000.0, 'g', 4)), QStringLiteral("sequenceAudio"));
        line(QStringLiteral("Start"), frameTimecode(std::llround(m_project.value(QStringLiteral("start")).toDouble() * fps), fps, drop),
             QStringLiteral("sequenceStart"));
        m_sections[format].summary = [canvas, fps] { return QStringLiteral("%1 × %2 · %3 fps").arg(canvas.width()).arg(canvas.height()).arg(QString::number(fps, 'g', 4)); };
    }
    // Tracks: each with its clip colour, id, name and clip count.
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    const int trackSection = addSection(into, QStringLiteral("tracks"), QStringLiteral("Tracks"), true);
    m_sections[trackSection].section->setCount(int(tracks.size()));
    for (const QJsonValue &value : tracks) {
        const QJsonObject track = value.toObject();
        auto *line = new QWidget;
        line->setObjectName(QStringLiteral("sequenceTrack"));
        line->setFixedHeight(InspectorRow::kHeight - 4);
        auto *layout = new QHBoxLayout(line);
        layout->setContentsMargins(0, 0, 0, 0);
        layout->setSpacing(8);
        auto *square = new KindSquare;
        // The timeline header's colour: a picture track of titles is ochre.
        square->setColor(theme.clip(trackKind(track)).header);
        layout->addWidget(square);
        const TrackBadge badge = trackBadge(m_project, track.value(QStringLiteral("id")).toString());
        auto *id = new QLabel(badge.badge);
        id->setFont(sizedFont(id->font(), 12, QFont::DemiBold));
        id->setFixedWidth(28);
        layout->addWidget(id);
        auto *name = plain(badge.name, QStringLiteral("sequenceTrackName"));
        layout->addWidget(name, 1);
        const int count = int(track.value(QStringLiteral("clips")).toArray().size());
        auto *clips = quietLabel(count == 1 ? QStringLiteral("1 clip") : QStringLiteral("%1 clips").arg(count));
        layout->addWidget(clips);
        sectionBody(trackSection)->addWidget(line);
    }
    m_sections[trackSection].summary = [tracks] {
        int video = 0, audio = 0;
        for (const QJsonValue &track : tracks) (isAudioTrack(track.toObject()) ? audio : video) += 1;
        return QStringLiteral("%1 video · %2 audio").arg(video).arg(audio);
    };
    // Markers: folded, with their count.
    const QJsonArray markers = m_project.value(QStringLiteral("markers")).toArray();
    const int markerSection = addSection(into, QStringLiteral("markers"), QStringLiteral("Markers"), false);
    m_sections[markerSection].section->setCount(int(markers.size()));
    for (const QJsonValue &value : markers) {
        const QJsonObject marker = value.toObject();
        auto *line = new QWidget;
        line->setObjectName(QStringLiteral("sequenceMarker"));
        line->setFixedHeight(InspectorRow::kHeight - 4);
        auto *layout = new QHBoxLayout(line);
        layout->setContentsMargins(0, 0, 0, 0);
        layout->setSpacing(8);
        auto *dot = new KindSquare;
        dot->setColor(theme.marker(marker.value(QStringLiteral("color")).toString()));
        layout->addWidget(dot);
        auto *time = plain(frameTimecode(std::llround(marker.value(QStringLiteral("time")).toDouble() * fps), fps, drop), QStringLiteral("markerTime"));
        time->setFixedWidth(84);
        layout->addWidget(time);
        const double duration = marker.value(QStringLiteral("duration")).toDouble();
        const QString label = marker.value(QStringLiteral("label")).toString();
        layout->addWidget(quietLabel(!label.isEmpty() ? label
                                     : duration > 0 ? QStringLiteral("Range · ") + frameTimecode(std::llround(duration * fps), fps, drop)
                                                    : QString()),
                          1);
        sectionBody(markerSection)->addWidget(line);
    }
    if (markers.isEmpty()) sectionBody(markerSection)->addWidget(quietLabel(QStringLiteral("No markers")));
}

// ---------------------------------------------------------------- masks

void InspectorPanel::buildMasksSection(QVBoxLayout *into) {
    if (!m_maskable || m_clips.size() != 1) return;
    const int section = addSection(into, QStringLiteral("masks"), QStringLiteral("Masks"));
    InspectorSection *masks = m_sections.at(section).section;
    masks->setObjectName(QStringLiteral("inspectorMasks"));
    for (QAction *tool : std::as_const(m_maskTools)) {
        auto *button = new QToolButton;
        button->setObjectName(QStringLiteral("maskTool"));
        button->setDefaultAction(tool);
        button->setAutoRaise(true);
        masks->addTrailing(button);
    }
    const QJsonArray list = m_clips.first().clip.value(QStringLiteral("masks")).toArray();
    masks->setCount(int(list.size()));
    if (list.isEmpty()) {
        auto *hint = quietLabel(QStringLiteral("Draw a mask on the picture to show only part of this clip."));
        hint->setObjectName(QStringLiteral("masksHint"));
        hint->setWordWrap(true);
        hint->setContentsMargins(0, 4, 0, 4);
        sectionBody(section)->addWidget(hint);
    }
    for (const QJsonValue &mask : list) buildMask(sectionBody(section), mask.toObject().value(QStringLiteral("name")).toString());
    m_sections[section].summary = [this] {
        QStringList shown;
        for (const QJsonValue &mask : m_clips.first().clip.value(QStringLiteral("masks")).toArray())
            shown << mask.toObject().value(QStringLiteral("name")).toString();
        return shown.join(QStringLiteral(", "));
    };
}

void InspectorPanel::buildMask(QVBoxLayout *into, const QString &name) {
    auto *box = new QWidget;
    box->setObjectName(QStringLiteral("maskBox"));
    box->setProperty("mask", name);
    auto *layout = new QVBoxLayout(box);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    auto *header = new QWidget;
    header->setFixedHeight(InspectorRow::kHeight);
    auto *row = new QHBoxLayout(header);
    row->setContentsMargins(0, 0, 0, 0);
    row->setSpacing(6);
    auto *chevron = new GlyphButton(GlyphButton::Glyph::Chevron);
    chevron->setObjectName(QStringLiteral("maskChevron"));
    row->addWidget(chevron);
    auto *shape = new QLabel;
    shape->setFixedSize(16, 16);
    const QJsonObject mask = [this, &name] {
        for (const QJsonValue &value : m_clips.first().clip.value(QStringLiteral("masks")).toArray())
            if (value.toObject().value(QStringLiteral("name")).toString() == name) return value.toObject();
        return QJsonObject{};
    }();
    const GlyphButton::Glyph glyph = mask.contains(QStringLiteral("ellipse")) ? GlyphButton::Glyph::Ellipse
                                     : mask.contains(QStringLiteral("points")) ? GlyphButton::Glyph::Pen
                                                                               : GlyphButton::Glyph::Rectangle;
    shape->setPixmap(glyphPixmap(QSize(16, 16), [glyph](QPainter &p, const QColor &color) {
        drawInspectorGlyph(p, glyph, QRectF(0, 0, 16, 16), color);
    }, editorTheme().text));
    row->addWidget(shape);
    auto *select = new QToolButton;
    select->setObjectName(QStringLiteral("maskName"));
    select->setText(name);
    select->setCheckable(true);
    select->setAutoRaise(true);
    select->setToolTip(QStringLiteral("Show this mask's handles on the Program monitor"));
    connect(select, &QToolButton::clicked, this, [this, name] {
        selectMask(name);
        emit maskSelected(name);
    });
    row->addWidget(select);
    row->addStretch(1);
    auto *inverted = new InspectorSwitch;
    inverted->setObjectName(QStringLiteral("maskInverted"));
    inverted->setToolTip(QStringLiteral("Show what is outside the mask"));
    connect(inverted, &QAbstractButton::clicked, this, [this, name](bool on) {
        emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("mask-set")}, {QStringLiteral("clip"), m_clips.first().id},
                                       {QStringLiteral("name"), name}, {QStringLiteral("inverted"), on}}});
    });
    row->addWidget(inverted);
    row->addWidget(quietLabel(QStringLiteral("Invert")));
    auto *more = new GlyphButton(GlyphButton::Glyph::More);
    more->setObjectName(QStringLiteral("maskMore"));
    auto *menu = new QMenu(box);
    menu->addAction(QStringLiteral("Ask Kerf about This Mask"), this, [this, name] {
        askAbout(QStringLiteral("@%1 mask %2").arg(displayName(m_clips.first()), name));
    })->setObjectName(QStringLiteral("maskAsk"));
    menu->addSeparator();
    menu->addAction(QStringLiteral("Remove"), this, [this, name] {
        emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("mask-remove")}, {QStringLiteral("clip"), m_clips.first().id},
                                       {QStringLiteral("name"), name}}});
    })->setObjectName(QStringLiteral("maskRemove"));
    connect(more, &QAbstractButton::clicked, this, [more, menu] { menu->popup(more->mapToGlobal(QPoint(0, more->height()))); });
    row->addWidget(more);
    layout->addWidget(header);
    // Opened: its mode, then feather, opacity and expansion.
    auto *body = new QWidget;
    body->setObjectName(QStringLiteral("maskBody"));
    auto *bodyLayout = new QVBoxLayout(body);
    bodyLayout->setContentsMargins(0, 0, 0, 4);
    bodyLayout->setSpacing(0);
    {
        const int r = addRow(bodyLayout, QStringLiteral("Mode"));
        auto *mode = popup(QStringLiteral("maskMode"));
        for (const QString &option : std::as_const(m_maskModes)) mode->addItem(option, option);
        mode->setToolTip(QStringLiteral("How the mask combines with the ones above it; none leaves a path for effects"));
        this->row(r)->controls()->addWidget(mode, 1);
        connect(mode, &QComboBox::activated, this, [this, name, mode](int index) {
            emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("mask-set")}, {QStringLiteral("clip"), m_clips.first().id},
                                           {QStringLiteral("name"), name}, {QStringLiteral("mode"), mode->itemText(index)}}});
        });
        m_refreshers.append([this, name, mode, inverted, select] {
            for (const QJsonValue &value : m_clips.first().clip.value(QStringLiteral("masks")).toArray()) {
                const QJsonObject current = value.toObject();
                if (current.value(QStringLiteral("name")).toString() != name) continue;
                const QSignalBlocker modeBlocker(mode);
                mode->setCurrentText(current.value(QStringLiteral("mode")).toString(QStringLiteral("add")));
                const QSignalBlocker invertBlocker(inverted);
                inverted->setChecked(current.value(QStringLiteral("inverted")).toBool());
            }
            const QSignalBlocker selectBlocker(select);
            select->setChecked(name == m_selectedMask);
        });
    }
    for (const QJsonValue &defValue : std::as_const(m_maskDefs)) {
        const QJsonObject def = defValue.toObject();
        const QString param = def.value(QStringLiteral("name")).toString();
        const int r = addRow(bodyLayout, presentationLabel(def));
        const Target target{Target::Kind::Mask, param, -1, name};
        if (def.value(QStringLiteral("type")).toString() == QStringLiteral("number")) {
            ScrubField *field = bindNumber(r, this->row(r)->controls(), target, def);
            field->setObjectName(QStringLiteral("maskParam_") + param);
            field->setFixedWidth(kSingleField);
            this->row(r)->controls()->addStretch(1);
        } else {
            bindValue(r, this->row(r)->controls(), target, def)->setObjectName(QStringLiteral("maskParam_") + param);
        }
    }
    layout->addWidget(body);
    const bool open = m_openMasks.contains(name) || name == m_selectedMask;
    body->setVisible(open);
    chevron->setOpen(open);
    connect(chevron, &QAbstractButton::clicked, this, [this, name, body, chevron] {
        const bool now = body->isHidden();
        body->setVisible(now);
        chevron->setOpen(now);
        if (now) m_openMasks.insert(name);
        else m_openMasks.remove(name);
    });
    into->addWidget(box);
}

// --------------------------------------------------------------- effects

void InspectorPanel::buildEffectsSection(QVBoxLayout *into, bool openByDefault) {
    if (m_clips.isEmpty()) return;
    // The catalog's effects are all picture effects for now: a sound clip
    // is offered none to add (its list still shows what it carries), and
    // one that carries none gets no Effects section at all, rather than a
    // row with nothing in it.
    const bool sound = kindKey(m_clips.first()) == QStringLiteral("audio");
    const bool offered = !sound || std::any_of(m_effectDefs.cbegin(), m_effectDefs.cend(), [](const QJsonObject &def) {
        return def.value(QStringLiteral("domain")).toString() == QStringLiteral("audio");
    });
    const bool carried = std::any_of(m_clips.cbegin(), m_clips.cend(), [](const Clip &clip) {
        return !clip.clip.value(QStringLiteral("effects")).toArray().isEmpty();
    });
    if (!offered && !carried) return;
    const int section = addSection(into, QStringLiteral("effects"), QStringLiteral("Effects"), openByDefault);
    InspectorSection *effects = m_sections.at(section).section;
    effects->setObjectName(QStringLiteral("inspectorEffects"));
    if (offered) {
        auto *add = new GlyphButton(GlyphButton::Glyph::Plus);
        add->setObjectName(QStringLiteral("effectsAdd"));
        add->setToolTip(QStringLiteral("Add an effect"));
        connect(add, &QAbstractButton::clicked, this, [this, sound] { emit addEffectRequested(sound); });
        effects->addTrailing(add);
    }
    if (m_clips.size() > 1) {
        // Several clips: which effects they carry, read-only.
        m_sections[section].summary = [this] {
            int carrying = 0;
            for (const Clip &clip : std::as_const(m_clips)) carrying += clip.clip.value(QStringLiteral("effects")).toArray().isEmpty() ? 0 : 1;
            return carrying == 0 ? QString() : QStringLiteral("On %1 of %2 clips").arg(carrying).arg(m_clips.size());
        };
        for (const Clip &clip : std::as_const(m_clips)) {
            QStringList names;
            for (const QJsonValue &effect : clip.clip.value(QStringLiteral("effects")).toArray()) {
                const QString fx = effect.toObject().value(QStringLiteral("fx")).toString();
                names << m_effectDefs.value(fx).value(QStringLiteral("name")).toString(fx);
            }
            if (names.isEmpty()) continue;
            auto *line = quietLabel(QStringLiteral("%1: %2").arg(displayName(clip), names.join(QStringLiteral(", "))));
            line->setWordWrap(true);
            line->setContentsMargins(0, 4, 0, 4);
            sectionBody(section)->addWidget(line);
        }
        if (offered) {
            auto *addAll = new QPushButton(QStringLiteral("Add effect to all"));
            addAll->setObjectName(QStringLiteral("addEffect"));
            addAll->setFlat(true);
            addAll->setProperty("inspectorLink", true);
            connect(addAll, &QPushButton::clicked, this, [this, sound] { emit addEffectRequested(sound); });
            sectionBody(section)->addWidget(addAll, 0, Qt::AlignLeft);
        }
        return;
    }
    const QJsonArray list = m_clips.first().clip.value(QStringLiteral("effects")).toArray();
    effects->setCount(int(list.size()));
    for (int index = 0; index < list.size(); ++index) buildEffect(sectionBody(section), index);
    if (offered) {
    auto *addEffect = new QPushButton(QStringLiteral("Add effect"));
    addEffect->setObjectName(QStringLiteral("addEffect"));
    addEffect->setFlat(true);
    addEffect->setProperty("inspectorLink", true);
    addEffect->setIcon(glyphIcon(QSize(12, 12), [](QPainter &p, const QColor &color) {
        drawInspectorGlyph(p, GlyphButton::Glyph::Plus, QRectF(0, 0, 12, 12), color);
    }));
    addEffect->setIconSize(QSize(12, 12));
    connect(addEffect, &QPushButton::clicked, this, [this, sound] { emit addEffectRequested(sound); });
    sectionBody(section)->addSpacing(4);
    sectionBody(section)->addWidget(addEffect, 0, Qt::AlignLeft);
    }
    // The effects that are on, by name, as many whole names as fit, the
    // ones switched off counted ("Primary Colour Correction · 1 off");
    // short of room, "1 on · 1 off". The tooltip names every one.
    m_sections[section].items = [this](InspectorSection *into) {
        QStringList on;
        QStringList all;
        int off = 0;
        for (const QJsonValue &effect : m_clips.first().clip.value(QStringLiteral("effects")).toArray()) {
            const QJsonObject e = effect.toObject();
            const QString fx = e.value(QStringLiteral("fx")).toString();
            QString name = m_effectDefs.value(fx).value(QStringLiteral("name")).toString();
            if (name.isEmpty()) name = e.value(QStringLiteral("doc")).toString(fx).section(QStringLiteral(": "), 0, 0).remove(QLatin1Char('.'));
            name = presentationEffectName(name);
            if (e.value(QStringLiteral("enabled")).toBool(true)) {
                on << name;
                all << name;
            } else {
                ++off;
                all << name + QStringLiteral(" (off)");
            }
        }
        InspectorSection::Summary summary;
        summary.items = on;
        summary.full = all.join(QStringLiteral(", "));
        if (off > 0) {
            summary.suffix = on.isEmpty() ? QStringLiteral("%1 off").arg(off) : QStringLiteral(" · %1 off").arg(off);
            summary.compact = on.isEmpty() ? summary.suffix : QStringLiteral("%1 on · %2 off").arg(on.size()).arg(off);
        } else if (!on.isEmpty()) {
            summary.compact = QStringLiteral("%1 on").arg(on.size());
        }
        into->setSummary(summary);
    };
}

void InspectorPanel::buildEffect(QVBoxLayout *into, int index) {
    const QJsonObject effect = m_clips.first().clip.value(QStringLiteral("effects")).toArray().at(index).toObject();
    const QString fx = effect.value(QStringLiteral("fx")).toString();
    const QJsonObject def = m_effectDefs.value(fx);
    const QString doc = effect.value(QStringLiteral("doc")).toString();
    QString name = def.value(QStringLiteral("name")).toString();
    if (name.isEmpty() && !doc.isEmpty()) name = doc.section(QStringLiteral(": "), 0, 0).remove(QLatin1Char('.'));
    if (name.isEmpty()) name = fx;
    else name = presentationEffectName(name);
    auto *box = new QWidget;
    box->setObjectName(QStringLiteral("effectBox"));
    box->setProperty("fx", fx);
    box->setProperty("index", index);
    auto *layout = new QVBoxLayout(box);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    auto *header = new QWidget;
    header->setObjectName(QStringLiteral("effectHeader"));
    header->setFixedHeight(InspectorRow::kHeight);
    header->setFocusPolicy(Qt::StrongFocus);
    header->setProperty("index", index);
    header->installEventFilter(this);
    auto *row = new QHBoxLayout(header);
    row->setContentsMargins(0, 0, 0, 0);
    row->setSpacing(6);
    auto *chevron = new GlyphButton(GlyphButton::Glyph::Chevron);
    chevron->setObjectName(QStringLiteral("effectChevron"));
    row->addWidget(chevron);
    auto *enabled = new PowerKey;
    enabled->setObjectName(QStringLiteral("effectEnabled"));
    enabled->setToolTip(def.value(QStringLiteral("doc")).toString(doc));
    connect(enabled, &QAbstractButton::clicked, this, [this, index](bool on) {
        emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-set")}, {QStringLiteral("clip"), m_clips.first().id},
                                       {QStringLiteral("index"), index}, {QStringLiteral("enabled"), on}}});
    });
    row->addWidget(enabled);
    auto *title = new QLabel(name);
    title->setObjectName(QStringLiteral("effectName"));
    title->setFont(sizedFont(title->font(), 13, QFont::DemiBold));
    title->setMinimumWidth(0);
    title->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    title->setToolTip(def.value(QStringLiteral("doc")).toString(doc));
    row->addWidget(title, 1);
    auto *more = new GlyphButton(GlyphButton::Glyph::More);
    more->setObjectName(QStringLiteral("effectMore"));
    more->setToolTip(QStringLiteral("More"));
    row->addWidget(more);
    layout->addWidget(header);
    // Folded: the values that differ from their defaults, on a second line.
    auto *summary = quietLabel(QString());
    summary->setObjectName(QStringLiteral("effectSummary"));
    summary->setContentsMargins(kEffectTextIndent, 0, 0, 6);
    summary->setMinimumWidth(0);
    summary->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    layout->addWidget(summary);
    auto *params = new QWidget;
    params->setObjectName(QStringLiteral("effectParams"));
    auto *paramsLayout = new QVBoxLayout(params);
    paramsLayout->setContentsMargins(0, 0, 0, 4);
    paramsLayout->setSpacing(0);
    auto *moreRows = new QWidget;
    moreRows->setObjectName(QStringLiteral("effectMoreRows"));
    auto *moreLayout = new QVBoxLayout(moreRows);
    moreLayout->setContentsMargins(0, 0, 0, 0);
    moreLayout->setSpacing(0);
    QList<int> rows;
    const QJsonArray defs = def.value(QStringLiteral("params")).toArray();
    // Groups ("Vignette: Amount") get quiet sub-headers; "N more" counts
    // groups when there are any, rows otherwise.
    QStringList groups;
    for (const QJsonValue &value : defs) {
        const QString group = presentationGroup(value.toObject());
        if (!group.isEmpty() && !groups.contains(group)) groups << group;
    }
    // Labels on one 12pt line: a first word the effect's name already says
    // goes ("Threshold" under Edge Glow), unless that would leave two rows
    // reading alike; the tooltip keeps the catalog's whole label.
    QStringList labels;
    for (const QJsonValue &value : defs) labels << presentationLabel(value.toObject());
    QStringList shortLabels;
    for (const QString &label : std::as_const(labels)) shortLabels << withoutSharedWord(label, name);
    for (qsizetype index = 0; index < shortLabels.size(); ++index)
        if (shortLabels.count(shortLabels.at(index)) > 1 || (labels.contains(shortLabels.at(index)) && shortLabels.at(index) != labels.at(index)))
            shortLabels[index] = labels.at(index);
    // Folding saves a click only when it hides two rows or more: with one
    // to spare, every row shows.
    const int rowsShown = defs.size() - kEffectRowsShown >= 2 ? kEffectRowsShown : int(defs.size());
    int shownRows = 0;
    int hidden = 0;
    QString lastGroup;
    int paramIndex = -1;
    for (const QJsonValue &value : defs) {
        const QJsonObject param = value.toObject();
        ++paramIndex;
        const QString group = presentationGroup(param);
        const bool inMore = groups.isEmpty() ? shownRows >= rowsShown : (!groups.isEmpty() && group != groups.first());
        QVBoxLayout *target = inMore ? moreLayout : paramsLayout;
        if (!group.isEmpty() && group != lastGroup) {
            auto *sub = quietLabel(group, 11);
            sub->setObjectName(QStringLiteral("effectGroup"));
            sub->setContentsMargins(0, 8, 0, 2);
            target->addWidget(sub);
            lastGroup = group;
            if (inMore) ++hidden;
        }
        if (inMore && groups.isEmpty()) ++hidden;
        else if (!inMore) ++shownRows;
        const QString paramName = param.value(QStringLiteral("name")).toString();
        const int r = addRow(target, shortLabels.at(paramIndex));
        const Target t{Target::Kind::Effect, paramName, index};
        const QString type = param.value(QStringLiteral("type")).toString();
        if (type == QStringLiteral("number")) {
            const bool bounded = param.contains(QStringLiteral("min")) && param.contains(QStringLiteral("max"));
            ScrubField *field = bindNumber(r, this->row(r)->controls(), t, param, bounded);
            field->setObjectName(QStringLiteral("effectParam_") + paramName);
            if (!bounded) {
                field->setFixedWidth(kSingleField);
                this->row(r)->controls()->addStretch(1);
            }
        } else {
            bindValue(r, this->row(r)->controls(), t, param)->setObjectName(QStringLiteral("effectParam_") + paramName);
        }
        this->row(r)->label()->setToolTip(britishSpelling(param.value(QStringLiteral("doc")).toString(paramName)));
        rows << r;
    }
    paramsLayout->addWidget(moreRows);
    const QString moreKey = QStringLiteral("editor/inspector/fx/%1/more").arg(fx);
    if (hidden > 0) {
        auto *toggle = new MoreToggle;
        toggle->setObjectName(QStringLiteral("effectMoreToggle"));
        const auto label = [toggle, hidden](bool open) {
            toggle->setText(open ? QStringLiteral("Fewer") : QStringLiteral("%1 more").arg(hidden));
            toggle->setOpen(open);
        };
        const bool open = QSettings().value(moreKey, false).toBool();
        moreRows->setVisible(open);
        label(open);
        connect(toggle, &QPushButton::clicked, this, [moreRows, label, moreKey] {
            const bool now = moreRows->isHidden();
            moreRows->setVisible(now);
            label(now);
            QSettings().setValue(moreKey, now);
        });
        auto *toggleRow = new QHBoxLayout;
        toggleRow->setContentsMargins(0, 4, 0, 0);
        toggleRow->addWidget(toggle);
        toggleRow->addStretch(1);
        paramsLayout->addLayout(toggleRow);
    } else {
        moreRows->hide();
    }
    if (def.isEmpty() && doc.isEmpty()) {
        auto *missing = quietLabel(QStringLiteral("Not in this version of Kerf"));
        missing->setObjectName(QStringLiteral("effectMissing"));
        missing->setContentsMargins(kEffectTextIndent, 0, 0, 6);
        layout->addWidget(missing);
    }
    layout->addWidget(params);
    const QString openKey = QStringLiteral("editor/inspector/fx/%1/open").arg(fx);
    const auto setOpen = [params, chevron, summary](bool open) {
        params->setVisible(open);
        chevron->setOpen(open);
        summary->setVisible(!open && !summary->text().isEmpty());
    };
    setOpen(QSettings().value(openKey, false).toBool() && !defs.isEmpty());
    connect(chevron, &QAbstractButton::clicked, this, [params, setOpen, openKey] {
        const bool now = params->isHidden();
        setOpen(now);
        QSettings().setValue(openKey, now);
    });
    auto *menu = new QMenu(box);
    menu->setObjectName(QStringLiteral("effectMenu"));
    menu->addAction(QStringLiteral("Reset Parameters"), this, [this, index] {
        const QJsonObject current = m_clips.first().clip.value(QStringLiteral("effects")).toArray().at(index).toObject();
        QJsonObject cleared;
        const QJsonObject params = current.value(QStringLiteral("params")).toObject();
        for (auto it = params.constBegin(); it != params.constEnd(); ++it) cleared.insert(it.key(), QJsonValue());
        for (const QJsonValue &param : m_effectDefs.value(current.value(QStringLiteral("fx")).toString()).value(QStringLiteral("params")).toArray())
            cleared.insert(param.toObject().value(QStringLiteral("name")).toString(), QJsonValue());
        if (cleared.isEmpty()) return;
        emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-set")}, {QStringLiteral("clip"), m_clips.first().id},
                                       {QStringLiteral("index"), index}, {QStringLiteral("params"), cleared}}});
    })->setObjectName(QStringLiteral("effectReset"));
    menu->addAction(QStringLiteral("Ask Kerf about This Effect"), this, [this, name] {
        askAbout(QStringLiteral("@%1 %2").arg(displayName(m_clips.first()), name));
    })->setObjectName(QStringLiteral("effectAsk"));
    menu->addSeparator();
    menu->addAction(QStringLiteral("Remove"), this, [this, index] {
        emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-remove")}, {QStringLiteral("clip"), m_clips.first().id},
                                       {QStringLiteral("index"), index}}});
    })->setObjectName(QStringLiteral("effectRemove"));
    connect(more, &QAbstractButton::clicked, this, [more, menu] { menu->popup(more->mapToGlobal(QPoint(0, more->height()))); });
    m_refreshers.append([this, index, enabled, title, summary, params, rows] {
        const QJsonObject current = m_clips.first().clip.value(QStringLiteral("effects")).toArray().at(index).toObject();
        const bool on = current.value(QStringLiteral("enabled")).toBool(true);
        {
            const QSignalBlocker blocker(enabled);
            enabled->setChecked(on);
        }
        if (title->property("off").toBool() == on) {
            title->setProperty("off", !on);
            title->style()->unpolish(title);
            title->style()->polish(title);
        }
        // Switched off: its name, second line and values (numbers, slider
        // fills, colour hex, file names) in the secondary ink, which still
        // reads; they still edit.
        for (ScrubField *field : params->findChildren<ScrubField *>()) field->setDimmed(!on);
        for (InspectorSlider *slider : params->findChildren<InspectorSlider *>()) slider->setDimmed(!on);
        for (QWidget *value : params->findChildren<QWidget *>()) {
            if (value->objectName() != QStringLiteral("colorHex") && !(qobject_cast<QLineEdit *>(value) && value->property("inspector").toBool()))
                continue;
            if (value->property("off").toBool() == !on) continue;
            value->setProperty("off", !on);
            value->style()->unpolish(value);
            value->style()->polish(value);
        }
        QStringList pieces;
        for (const int r : rows)
            for (const Binding &binding : m_rows.at(r).values)
                if (binding.summary)
                    if (const QString piece = binding.summary(); !piece.isEmpty()) pieces << piece;
        summary->setText(pieces.join(QStringLiteral(" · ")));
        summary->setToolTip(summary->text());
        summary->setVisible(params->isHidden() && !pieces.isEmpty());
    });
    m_effectBoxes.append(box);
    into->addWidget(box);
}

// ------------------------------------------------------------- drag & drop

int InspectorPanel::effectIndexAt(int y) const {
    // Before the first effect whose middle is below `y`.
    for (int index = 0; index < m_effectBoxes.size(); ++index) {
        const QWidget *box = m_effectBoxes.at(index);
        if (!box->isVisible()) return int(m_effectBoxes.size());
        const int middle = box->mapTo(m_content, QPoint(0, box->findChild<QWidget *>(QStringLiteral("effectHeader"))->height() / 2)).y();
        if (y < middle) return index;
    }
    return int(m_effectBoxes.size());
}

void InspectorPanel::showDropLine(int index) {
    m_dropIndex = index;
    if (index < 0 || m_effectBoxes.isEmpty() || !m_effectBoxes.first()->isVisible()) {
        m_dropLine->hide();
        return;
    }
    const QWidget *anchor = index < m_effectBoxes.size() ? m_effectBoxes.at(index) : m_effectBoxes.last();
    const QPoint top = anchor->mapTo(m_content, QPoint(0, index < m_effectBoxes.size() ? 0 : anchor->height()));
    m_dropLine->setGeometry(kSide, top.y() - 1, m_content->width() - 2 * kSide, 2);
    m_dropLine->raise();
    m_dropLine->show();
}

void InspectorPanel::dragEnterEvent(QDragEnterEvent *event) {
    if (m_mode == Mode::Clip && event->mimeData()->hasFormat(QString::fromLatin1(kFxMime))) event->acceptProposedAction();
}

void InspectorPanel::dragMoveEvent(QDragMoveEvent *event) {
    if (m_mode != Mode::Clip || !event->mimeData()->hasFormat(QString::fromLatin1(kFxMime))) return;
    showDropLine(effectIndexAt(m_content->mapFrom(this, event->position().toPoint()).y()));
    event->acceptProposedAction();
}

void InspectorPanel::dragLeaveEvent(QDragLeaveEvent *) {
    showDropLine(-1);
}

void InspectorPanel::dropEvent(QDropEvent *event) {
    if (m_mode != Mode::Clip || !event->mimeData()->hasFormat(QString::fromLatin1(kFxMime))) return;
    const QString fx = QString::fromUtf8(event->mimeData()->data(QString::fromLatin1(kFxMime)));
    const int index = effectIndexAt(m_content->mapFrom(this, event->position().toPoint()).y());
    showDropLine(-1);
    if (fx.isEmpty()) return;
    QJsonObject op{{QStringLiteral("op"), QStringLiteral("fx-add")}, {QStringLiteral("clip"), m_clips.first().id}, {QStringLiteral("fx"), fx}};
    if (index < m_effectBoxes.size()) op.insert(QStringLiteral("index"), index);
    m_revealEffect = index;
    emitOps(QJsonArray{op});
    event->acceptProposedAction();
}

void InspectorPanel::showEvent(QShowEvent *event) {
    // The Program frame kept coming while it was hidden.
    if (m_thumbnail) m_thumbnail->setImages(thumbnails());
    QWidget::showEvent(event);
}

ScrubField *InspectorPanel::newScrubField() {
    auto *field = new ScrubField;
    connect(field, &ScrubField::doneEditing, this, &InspectorPanel::focusReturnRequested);
    return field;
}

void InspectorPanel::keyPressEvent(QKeyEvent *event) {
    // Esc that nothing in the Inspector took (no field being typed in)
    // hands the keys back to the timeline, so the transport works again.
    if (event->key() == Qt::Key_Escape && event->modifiers() == Qt::NoModifier) {
        emit focusReturnRequested();
        event->accept();
        return;
    }
    QWidget::keyPressEvent(event);
}

void InspectorPanel::showScrollBar(bool shown) {
    if (m_scroll->property("active").toBool() == shown) return;
    m_scroll->setProperty("active", shown);
    QScrollBar *bar = m_scroll->verticalScrollBar();
    bar->style()->unpolish(bar);
    bar->style()->polish(bar);
    bar->update();
}

void InspectorPanel::enterEvent(QEnterEvent *event) {
    QWidget::enterEvent(event);
    showScrollBar(true);
}

void InspectorPanel::leaveEvent(QEvent *event) {
    QWidget::leaveEvent(event);
    if (!m_scrollBarTimer->isActive()) showScrollBar(false);
}

void InspectorPanel::fitScrollBar() {
    // While the body scrolls, its scroll bar sits in the right padding, so
    // its rows keep the header's right edge.
    QScrollBar *bar = m_scroll->verticalScrollBar();
    const bool takesRoom = bar->isVisibleTo(m_scroll) && !bar->style()->styleHint(QStyle::SH_ScrollBar_Transient, nullptr, bar);
    const int right = takesRoom ? std::max(0, kPlateInset - bar->sizeHint().width()) : kPlateInset;
    if (m_contentLayout->contentsMargins().right() == right) return;
    m_contentLayout->setContentsMargins(kPlateInset, 0, right, 0);
    m_contentLayout->activate();
}

bool InspectorPanel::eventFilter(QObject *watched, QEvent *event) {
    if (watched == m_scroll && event->type() == QEvent::Resize) m_scrollEdge->setGeometry(0, 0, m_scroll->width(), 1);
    if (watched == m_scroll->verticalScrollBar() && (event->type() == QEvent::Show || event->type() == QEvent::Hide)) fitScrollBar();
    if (watched == m_scroll->viewport() && event->type() == QEvent::Wheel) {
        showScrollBar(true);
        m_scrollBarTimer->start();
    }
    auto *widget = qobject_cast<QWidget *>(watched);
    if (widget && widget->objectName() == QStringLiteral("effectHeader") && event->type() == QEvent::KeyPress) {
        const int code = static_cast<QKeyEvent *>(event)->key();
        if ((code == Qt::Key_Delete || code == Qt::Key_Backspace) && m_mode == Mode::Clip) {
            emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-remove")}, {QStringLiteral("clip"), m_clips.first().id},
                                           {QStringLiteral("index"), widget->property("index").toInt()}}});
            return true;
        }
    }
    if (widget && widget->objectName() == QStringLiteral("effectHeader") && event->type() == QEvent::Paint && widget->hasFocus()) {
        QPainter painter(widget);
        painter.setPen(QPen(editorTheme().focusRing, 1.0));
        painter.drawRoundedRect(QRectF(widget->rect()).adjusted(0.5, 0.5, -0.5, -0.5), 5, 5);
    }
    // The title's text commits on ⌘Return or when it loses focus.
    if (auto *edit = qobject_cast<QPlainTextEdit *>(watched); edit && edit->property("commit").toBool() && m_mode == Mode::Clip) {
        const auto commit = [this, edit] {
            commitValue(Target{Target::Kind::Prop, QStringLiteral("text")}, defNamed(m_clips.first(), QStringLiteral("text")), edit->toPlainText());
        };
        if (event->type() == QEvent::FocusOut) commit();
        if (event->type() == QEvent::KeyPress) {
            auto *key = static_cast<QKeyEvent *>(event);
            if ((key->key() == Qt::Key_Return || key->key() == Qt::Key_Enter) && (key->modifiers() & Qt::ControlModifier)) {
                commit();
                return true;
            }
            if (key->key() == Qt::Key_Escape) {
                refreshValues();
                edit->clearFocus();
                emit focusReturnRequested();
                return true;
            }
        }
    }
    return QWidget::eventFilter(watched, event);
}

} // namespace kerf
