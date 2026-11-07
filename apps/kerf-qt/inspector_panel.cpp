#include "inspector_panel.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "icons.h"
#include "inspector_widgets.h"
#include "media_artwork.h"

#include <QAction>
#include <QApplication>
#include <QColorDialog>
#include <QComboBox>
#include <QDragEnterEvent>
#include <QElapsedTimer>
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
#include <QLinearGradient>
#include <QMap>
#include <QMenu>
#include <QMimeData>
#include <QPainter>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QRegularExpression>
#include <QScrollArea>
#include <QScrollBar>
#include <QSet>
#include <QSettings>
#include <QSignalBlocker>
#include <QTextDocument>
#include <QTimer>
#include <QToolButton>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>
#include <limits>

namespace kerf {

namespace {

// Effects show this many parameters before "N more" (N is 2 or more).
constexpr int kEffectRowsShown = 5;
// The clip header (spec 5.1): 44 tall, pad 4 8 4 12.
constexpr int kHeaderHeight = 44;
constexpr int kFooterHeight = 32;
constexpr int kFadeHeight = 12;
const char *kFxMime = "application/x-kerf-fx";
// The effects the design gives groups of their own.
const char *kCornerPin = "distort.cornerpin";
const char *kStabilizer = "distort.warpstabilizer";
const char *kChannelEq = "audio.eq";
const char *kCdl = "color.asccdl";
const char *kLumetri = "color.lumetri";

QColor col(QRgb rgba) {
    return tokens::color(rgba);
}

QString css(QRgb rgba) {
    const QColor c = col(rgba);
    return QStringLiteral("rgba(%1,%2,%3,%4)").arg(c.red()).arg(c.green()).arg(c.blue()).arg(c.alphaF(), 0, 'f', 3);
}

QString key(const char *text) {
    return QString::fromLatin1(text);
}

QString fx(const char *id) {
    return QString::fromLatin1(id);
}

QLabel *styledLabel(const QString &text, const tokens::TypeStyle &style, QRgb ink) {
    auto *label = new QLabel(text);
    label->setFont(tokens::font(style));
    label->setStyleSheet(QStringLiteral("QLabel { color: %1; background: transparent; %2 }").arg(css(ink), inspectorFontCss(style)));
    return label;
}

// A note or a list's quiet line: UI 11 ink2.
QLabel *quietLabel(const QString &text) {
    QLabel *label = styledLabel(text, tokens::meta, tokens::ink2);
    label->setObjectName(QStringLiteral("inspectorQuiet"));
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

QString ratioText(double ratio) {
    QString text = QString::number(std::abs(ratio), 'f', 4);
    while (text.endsWith(QLatin1Char('0'))) text.chop(1);
    if (text.endsWith(QLatin1Char('.'))) text.chop(1);
    return text;
}

// A generic family ("sans-serif") as the family it resolves to here.
QString resolvedFamily(const QString &generic) {
    QFont font(generic);
    font.setStyleHint(generic == QStringLiteral("serif") ? QFont::Serif : generic == QStringLiteral("monospace") ? QFont::Monospace
                                                                                                               : QFont::SansSerif);
    const QString family = QFontInfo(font).family();
    return family.isEmpty() || family.startsWith(QLatin1Char('.')) ? QStringLiteral("System") : family;
}

// The installed families, fetched only when the pop-up opens.
class FontPopup final : public GhostPopup {
public:
    using GhostPopup::GhostPopup;
    void showPopup() override {
        if (!m_filled) {
            m_filled = true;
            const QString current = currentText();
            const QSignalBlocker blocker(this);
            for (const QString &family : QFontDatabase::families())
                if (findText(family) < 0) insertItem(count() - 1, family);
            setCurrentText(current);
        }
        GhostPopup::showPopup();
    }

private:
    bool m_filled = false;
};

GhostPopup *popup(const QString &name) {
    auto *combo = new GhostPopup;
    combo->setObjectName(name);
    return combo;
}

QPixmap swatch(const QString &color, const QSize &size) {
    QPixmap pixmap(size * 2);
    pixmap.setDevicePixelRatio(2);
    pixmap.fill(Qt::transparent);
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    // Ringed in track2, which reads at 3:1 on white and black alike, so
    // either swatch shows on the dark column.
    painter.setPen(QPen(col(tokens::track2), 1));
    painter.setBrush(modelColor(color));
    painter.drawRoundedRect(QRectF(QPointF(0.5, 0.5), QSizeF(size) - QSizeF(1, 1)), tokens::radius::small - 0.5, tokens::radius::small - 0.5);
    return pixmap;
}

// A plain pane that draws a hairline at its top or its foot (inset 0 ±1px 0).
class LinePane final : public QWidget {
public:
    LinePane(bool top, bool bottom) : m_top(top), m_bottom(bottom) {}

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter painter(this);
        if (m_top) painter.fillRect(QRect(0, 0, width(), 1), col(tokens::line));
        if (m_bottom) painter.fillRect(QRect(0, height() - 1, width(), 1), col(tokens::line));
    }

private:
    bool m_top;
    bool m_bottom;
};

// A ghost link button (.ghost-b) that is a push button, for the rows that
// add, reveal or ask: UI 12 ink2, hover raised and ink, an optional icon.
class LinkButton final : public QPushButton {
public:
    explicit LinkButton(const QString &text, const QString &icon = QString(), int height = tokens::height::h24,
                        const tokens::TypeStyle &style = tokens::control)
        : QPushButton(text), m_icon(icon), m_style(style) {
        setAttribute(Qt::WA_Hover);
        setFlat(true);
        setCursor(Qt::PointingHandCursor);
        setFocusPolicy(Qt::TabFocus);
        setFixedHeight(height);
        setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed);
        setFont(tokens::font(style));
        setStyleSheet(QStringLiteral("QPushButton { %1 }").arg(inspectorFontCss(style)));
    }
    QSize sizeHint() const override {
        const int icon = m_icon.isEmpty() ? 0 : 16 + 4;
        return {8 + icon + ui::snapWidth(m_style, text()) + 8, height()};
    }
    QSize minimumSizeHint() const override { return sizeHint(); }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        const bool hovered = underMouse() && isEnabled();
        if (hovered || isDown()) {
            painter.setPen(Qt::NoPen);
            painter.setBrush(col(isDown() ? tokens::hover2 : tokens::raised));
            painter.drawRoundedRect(QRectF(rect()), tokens::radius::control, tokens::radius::control);
        }
        const QColor ink = !isEnabled() ? col(tokens::dis) : hovered ? col(tokens::ink) : col(tokens::ink2);
        double left = 8;
        if (!m_icon.isEmpty()) {
            icons::paint(painter, m_icon, QRectF(left, (height() - 16) / 2.0, 16, 16), ink, 16);
            left += 20;
        }
        painter.setFont(font());
        painter.setPen(ink);
        painter.drawText(QRectF(left, 0, width() - left - 4, height()), Qt::AlignLeft | Qt::AlignVCenter, text());
        if (hasFocus()) ui::paintFocusRing(painter, QRectF(rect()), tokens::radius::control);
    }

private:
    QString m_icon;
    tokens::TypeStyle m_style;
};

// The body's 12-point bottom fade (the mockup's .ibody mask) over the
// scroll area's viewport, while there is more below.
class FadeOverlay final : public QWidget {
public:
    using QWidget::QWidget;

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter painter(this);
        QLinearGradient fade(0, 0, 0, height());
        QColor clear = col(tokens::chrome);
        clear.setAlpha(0);
        fade.setColorAt(0, clear);
        fade.setColorAt(1, col(tokens::chrome));
        painter.fillRect(rect(), fade);
    }
};

// "4s18f", "1m04s18f": a duration as the clip header's meta line says it.
QString durationText(qint64 frames, double fps) {
    const qint64 rate = std::max<qint64>(1, std::llround(fps));
    const qint64 seconds = frames / rate;
    const qint64 rest = frames % rate;
    if (seconds >= 60)
        return QStringLiteral("%1m%2s%3f").arg(seconds / 60).arg(seconds % 60, 2, 10, QLatin1Char('0')).arg(rest);
    return QStringLiteral("%1s%2f").arg(seconds).arg(rest);
}

// "25p", "23.976p".
QString rateText(double fps) {
    if (fps <= 0) return {};
    return QString::number(fps, 'g', 5) + QLatin1Char('p');
}

QString channelsText(int channels) {
    if (channels == 1) return QStringLiteral("mono");
    if (channels == 2) return QStringLiteral("stereo");
    return channels > 0 ? QStringLiteral("%1 ch").arg(channels) : QString();
}

QString kiloHertz(double rate) {
    return rate > 0 ? QStringLiteral("%1 kHz").arg(QString::number(rate / 1000.0, 'g', 4)) : QString();
}

// A codec as people name it, with its profile: "ProRes 4444", "H.264 High".
QString codecText(const QJsonObject &info) {
    static const QHash<QString, QString> names{
        {QStringLiteral("prores"), QStringLiteral("ProRes")},   {QStringLiteral("h264"), QStringLiteral("H.264")},
        {QStringLiteral("hevc"), QStringLiteral("HEVC")},       {QStringLiteral("av1"), QStringLiteral("AV1")},
        {QStringLiteral("vp9"), QStringLiteral("VP9")},         {QStringLiteral("dnxhd"), QStringLiteral("DNxHD")},
        {QStringLiteral("mjpeg"), QStringLiteral("Motion JPEG")}, {QStringLiteral("png"), QStringLiteral("PNG")},
        {QStringLiteral("aac"), QStringLiteral("AAC")},         {QStringLiteral("pcm_s24le"), QStringLiteral("PCM")},
        {QStringLiteral("pcm_s16le"), QStringLiteral("PCM")},
    };
    const QString codec = info.value(QStringLiteral("codec")).toString();
    if (codec.isEmpty()) return {};
    QString text = names.value(codec.toLower(), codec);
    const QString profile = info.value(QStringLiteral("profile")).toString();
    if (!profile.isEmpty() && profile.compare(QStringLiteral("unknown"), Qt::CaseInsensitive) != 0) text += QLatin1Char(' ') + profile;
    return text;
}

// The colour the file is tagged with: "Rec.709", "Rec.2020 · HLG". Untagged is empty.
QString colourText(const QJsonObject &info) {
    static const QHash<QString, QString> primaries{
        {QStringLiteral("bt709"), QStringLiteral("Rec.709")},     {QStringLiteral("bt2020"), QStringLiteral("Rec.2020")},
        {QStringLiteral("smpte432"), QStringLiteral("P3-D65")},   {QStringLiteral("smpte431"), QStringLiteral("DCI-P3")},
        {QStringLiteral("bt470bg"), QStringLiteral("Rec.601 PAL")}, {QStringLiteral("smpte170m"), QStringLiteral("Rec.601 NTSC")},
    };
    static const QHash<QString, QString> transfers{
        {QStringLiteral("bt709"), QStringLiteral("Rec.709")},         {QStringLiteral("arib-std-b67"), QStringLiteral("HLG")},
        {QStringLiteral("smpte2084"), QStringLiteral("PQ")},          {QStringLiteral("iec61966-2-1"), QStringLiteral("sRGB")},
        {QStringLiteral("linear"), QStringLiteral("Linear")},         {QStringLiteral("bt2020-10"), QStringLiteral("Rec.2020")},
        {QStringLiteral("smpte170m"), QStringLiteral("Rec.601")},
    };
    const QString p = info.value(QStringLiteral("primaries")).toString();
    const QString t = info.value(QStringLiteral("transfer")).toString();
    const QString s = info.value(QStringLiteral("colorSpace")).toString();
    QStringList parts;
    const QString named = primaries.value(p, p.isEmpty() ? primaries.value(s, s) : p);
    if (!named.isEmpty()) parts << named;
    const QString transfer = transfers.value(t, t);
    if (!transfer.isEmpty() && transfer != named) parts << transfer;
    return parts.join(QStringLiteral(" · "));
}

// A slider's travel for a row: the catalog's range, or the design's for
// the clip's own properties whose catalog range is too wide to steer
// (scale 0–250%, rotation ±180°).
void sliderTravel(const QString &name, const NumberFormat &format, double *minimum, double *maximum) {
    *minimum = format.min;
    *maximum = format.max;
    if (name == QStringLiteral("scale") || name == QStringLiteral("scale_x") || name == QStringLiteral("scale_y")) {
        *minimum = 0.0;
        *maximum = 250.0;
    } else if (name == QStringLiteral("rotation") || name == QStringLiteral("rotate_x") || name == QStringLiteral("rotate_y")) {
        *minimum = -180.0;
        *maximum = 180.0;
    }
    if (*minimum < -1e8) *minimum = -100.0;
    if (*maximum > 1e8) *maximum = 100.0;
}

QString targetKey(const InspectorPanel::Target &target) {
    using Kind = InspectorPanel::Target::Kind;
    switch (target.kind) {
    case Kind::Prop: return QStringLiteral("prop:") + target.name;
    case Kind::Linked: return QStringLiteral("linked:") + target.name;
    case Kind::Effect: return QStringLiteral("fx:%1:%2").arg(target.index).arg(target.name);
    case Kind::Mask: return QStringLiteral("mask:%1:%2").arg(target.mask, target.name);
    case Kind::Fx: return QStringLiteral("fxid:%1:%2").arg(target.fx, target.name);
    }
    return {};
}

// The colour wheels: what each pushes, in ASC CDL and in Lumetri.
struct WheelSpec {
    const char *label;
    const char *cdl;     // r_<cdl>, g_<cdl>, b_<cdl>
    double neutral;      // the CDL value that changes nothing
    double range;        // the chroma at the wheel's edge
    bool deviation;      // shown as the change from neutral (Lift, Gamma)
    const char *lumetri; // wheel_<lumetri>_x/_y/_l
};
constexpr WheelSpec kWheels[3] = {
    {"Lift", "offset", 0.0, 0.25, true, "shadows"},
    {"Gamma", "power", 1.0, 0.5, true, "midtones"},
    {"Gain", "slope", 1.0, 0.5, false, "highlights"},
};

// Rec.709 luma and chroma of an RGB triple, and back: the master value is
// the luma, the puck the chroma (x Cb, y −Cr, as a vectorscope reads it).
void toWheel(double r, double g, double b, double neutral, double range, double *master, QPointF *puck) {
    const double y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    const double cb = (b - y) / 1.8556;
    const double cr = (r - y) / 1.5748;
    *master = y;
    Q_UNUSED(neutral)
    *puck = QPointF(cb / range, -cr / range);
}

void fromWheel(double master, const QPointF &puck, double range, double *r, double *g, double *b) {
    const double cb = puck.x() * range;
    const double cr = -puck.y() * range;
    *r = master + 1.5748 * cr;
    *b = master + 1.8556 * cb;
    *g = (master - 0.2126 * *r - 0.0722 * *b) / 0.7152;
}

double round4(double value) {
    return std::round(value * 10000.0) / 10000.0;
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
    setStyleSheet(QStringLiteral("QWidget#inspector { background: %1; }").arg(css(tokens::chrome)));
    setAcceptDrops(true);
    auto *outer = new QVBoxLayout(this);
    outer->setContentsMargins(0, 0, 0, 0);
    outer->setSpacing(0);
    m_scroll = new QScrollArea;
    m_scroll->setObjectName(QStringLiteral("inspectorScroll"));
    // A click in the Inspector leaves the keys where they were (the
    // timeline's transport); only its fields take them, to be typed in.
    m_scroll->setFocusPolicy(Qt::NoFocus);
    m_scroll->setFrameShape(QFrame::NoFrame);
    m_scroll->setWidgetResizable(true);
    // The body scrolls with its scroll bar hidden (spec 5).
    m_scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_scroll->setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_scroll->setStyleSheet(QStringLiteral("QScrollArea#inspectorScroll { background: transparent; border: none; }"
                                           "QWidget#inspectorContent { background: transparent; }"));
    m_scroll->viewport()->setAutoFillBackground(false);
    outer->addWidget(m_scroll, 1);
    m_content = new QWidget;
    m_content->setObjectName(QStringLiteral("inspectorContent"));
    m_scroll->setWidget(m_content);
    m_contentLayout = new QVBoxLayout(m_content);
    m_contentLayout->setContentsMargins(0, 0, 0, 0);
    m_contentLayout->setSpacing(0);
    m_header = new LinePane(false, true);
    m_header->setObjectName(QStringLiteral("inspectorHeader"));
    m_header->setFixedHeight(kHeaderHeight);
    m_contentLayout->addWidget(m_header);
    m_note = quietLabel(QString());
    m_note->setObjectName(QStringLiteral("inspectorNote"));
    m_note->setWordWrap(true);
    m_note->setContentsMargins(12, 8, 12, 8);
    m_note->hide();
    m_contentLayout->addWidget(m_note);
    m_contentLayout->addStretch(1);
    m_content->installEventFilter(this);
    m_fade = new FadeOverlay(m_scroll);
    m_fade->setObjectName(QStringLiteral("inspectorFade"));
    m_fade->setAttribute(Qt::WA_TransparentForMouseEvents);
    m_fade->hide();
    m_scroll->installEventFilter(this);
    connect(m_scroll->verticalScrollBar(), &QScrollBar::valueChanged, this, [this](int value) {
        if (!m_kindForSections.isEmpty()) m_scrollByKind.insert(m_kindForSections, value);
        updateFade();
    });
    connect(m_scroll->verticalScrollBar(), &QScrollBar::rangeChanged, this, &InspectorPanel::updateFade);
    m_dropLine = new QFrame(m_content);
    m_dropLine->setObjectName(QStringLiteral("inspectorDropLine"));
    m_dropLine->setFixedHeight(2);
    m_dropLine->setStyleSheet(QStringLiteral("QFrame#inspectorDropLine { background: %1; border: 0; }").arg(css(tokens::accent)));
    m_dropLine->hide();
    // The footer (spec 5.1): 32, a line at its top, Save effects preset at its right.
    m_footer = new LinePane(true, false);
    m_footer->setObjectName(QStringLiteral("inspectorFooter"));
    m_footer->setFixedHeight(kFooterHeight);
    auto *footer = new QHBoxLayout(m_footer);
    footer->setContentsMargins(12, 0, 12, 0);
    footer->addStretch(1);
    m_savePreset = new ui::Button(QStringLiteral("Save effects preset"), ui::Button::Kind::Secondary);
    m_savePreset->setObjectName(QStringLiteral("saveEffectsPreset"));
    connect(m_savePreset, &QAbstractButton::clicked, this, [this] {
        if (m_mode == Mode::Clip && !m_clips.isEmpty()) emit savePresetRequested(m_clips.first().id);
    });
    footer->addWidget(m_savePreset);
    outer->addWidget(m_footer);
    // ⌥⌘R resets the selected clip's properties.
    auto *resetAllAction = new QAction(QStringLiteral("Reset all properties"), this);
    resetAllAction->setObjectName(QStringLiteral("inspectorResetAll"));
    resetAllAction->setShortcut(QKeySequence(Qt::CTRL | Qt::ALT | Qt::Key_R));
    resetAllAction->setShortcutContext(Qt::WindowShortcut);
    connect(resetAllAction, &QAction::triggered, this, &InspectorPanel::resetAll);
    addAction(resetAllAction);
    buildHeader();
    refresh();
}

InspectorPanel::~InspectorPanel() {
    delete m_parking;
}

// ------------------------------------------------------------------ state

void InspectorPanel::setState(const QJsonObject &timeline, const QStringList &clips) {
    QStringList selection = clips;
    selection.removeAll(QString());
    m_project = timeline;
    m_selection = selection;
    refresh();
}

void InspectorPanel::setSelection(const QStringList &clips) {
    setState(m_project, clips);
}

void InspectorPanel::setPane(Pane pane) {
    if (m_pane == pane) return;
    m_pane = pane;
    refresh();
}

void InspectorPanel::setFinish(bool finish) {
    if (m_finish == finish) return;
    m_finish = finish;
    refresh();
}

void InspectorPanel::setPlayhead(double seconds) {
    if (std::abs(seconds - m_playhead) < 1e-9) return;
    m_playhead = seconds;
    // Animated values follow the playhead, and the keyframe buttons say
    // whether there is a key under it.
    bool animated = false;
    for (const RowEntry &entry : std::as_const(m_rows)) animated |= entry.row->isAnimated();
    if (animated) {
        refreshValues();
        return;
    }
    for (int index = 0; index < m_rows.size(); ++index)
        if (!m_rows.at(index).keyed.isEmpty()) refreshKeyframe(index);
}

void InspectorPanel::setArtwork(MediaArtwork *artwork) {
    m_artwork = artwork;
}

void InspectorPanel::setProgramFrame(const QImage &frame) {
    m_programFrame = frame;
}

void InspectorPanel::setEffectCatalog(const QJsonArray &catalog) {
    for (const QJsonValue &value : catalog) {
        const QJsonObject entry = value.toObject();
        const QString id = entry.value(QStringLiteral("id")).toString();
        // The fuller entry wins (kerf_fx_catalog_all names the domain).
        QJsonObject merged = m_effectDefs.value(id);
        for (auto it = entry.constBegin(); it != entry.constEnd(); ++it) merged.insert(it.key(), it.value());
        m_effectDefs.insert(id, merged);
    }
    forgetBodies();
    refresh();
}

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

void InspectorPanel::setProposals(const QJsonArray &proposals, const QJsonObject &timelines) {
    if (proposals == m_proposals && timelines == m_proposalTimelines) return;
    m_proposals = proposals;
    m_proposalTimelines = timelines;
    placeProposalHint();
}

void InspectorPanel::setThreadResolver(std::function<QString(const QString &)> threadOf) {
    m_threadOf = std::move(threadOf);
}

void InspectorPanel::setMediaInfo(const QJsonObject &media) {
    bool changed = false;
    for (auto it = media.constBegin(); it != media.constEnd(); ++it) {
        if (m_mediaInfo.value(it.key()) == it.value()) continue;
        m_mediaInfo.insert(it.key(), it.value());
        changed = true;
    }
    if (changed) refresh();
}

void InspectorPanel::setPresetSaving(bool available, const QString &reason) {
    m_presetSaving = available;
    m_presetReason = reason;
    refreshFooter();
}

QString InspectorPanel::kindText() const {
    return metaText();
}

QString InspectorPanel::nameText() const {
    return m_name ? m_name->name() : QString();
}

QString InspectorPanel::metaText() const {
    return m_kind ? m_kind->property("full").toString() : QString();
}

QString InspectorPanel::proposalHintText() const {
    return m_hint && !m_hint->isHidden() && m_hintText ? m_hintText->text() : QString();
}

QString InspectorPanel::infoValue(const QString &name) const {
    return m_info.value(name);
}

QStringList InspectorPanel::infoKeys() const {
    QStringList keys;
    for (QLabel *label : m_content->findChildren<QLabel *>(QStringLiteral("infoKey"))) keys << label->text();
    return keys;
}

InspectorSection *InspectorPanel::section(const QString &id) const {
    for (const SectionEntry &entry : m_sections)
        if (entry.section->id() == id) return entry.section;
    return nullptr;
}

InspectorRow *InspectorPanel::row(int index) const {
    return m_rows.at(index).row;
}

std::optional<InspectorPanel::Clip> InspectorPanel::soundOf(const Clip &clip) const {
    if (clip.track.value(QStringLiteral("type")).toString() == QStringLiteral("audio")) return clip;
    if (clip.linked.isEmpty()) return std::nullopt;
    QHash<QString, QJsonObject> sources;
    for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray())
        sources.insert(value.toObject().value(QStringLiteral("id")).toString(), value.toObject());
    Clip sound{clip.linked.value(QStringLiteral("id")).toString(), clip.linked, clip.linkedTrack,
               sources.value(clip.linked.value(QStringLiteral("sourceId")).toString()), clip.clip, clip.track};
    return sound;
}

void InspectorPanel::gatherClips() {
    m_clips.clear();
    m_silent = false;
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
    // The Audio pane edits the sound: a sound clip's own, or the clip
    // linked to a picture clip.
    if (m_pane == Pane::Audio && !m_clips.isEmpty()) {
        QList<Clip> sounds;
        QSet<QString> seen;
        for (const Clip &clip : std::as_const(m_clips))
            if (const std::optional<Clip> sound = soundOf(clip); sound && !seen.contains(sound->id)) {
                seen.insert(sound->id);
                sounds << *sound;
            }
        m_silent = sounds.isEmpty();
        m_clips = sounds;
    }
    if (m_clips.isEmpty()) {
        m_mode = m_silent ? Mode::MixedKinds : Mode::Sequence;
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
        const QString generator = clip.clip.value(QStringLiteral("fx")).toString();
        if (generator == QStringLiteral("title") || generator == QStringLiteral("solid") || generator == QStringLiteral("adjustment"))
            return generator;
        return QStringLiteral("generator:") + generator;
    }
    if (kind == QStringLiteral("sequence")) return QStringLiteral("sequence");
    return QStringLiteral("video");
}

QString InspectorPanel::kindName(const Clip &clip) const {
    const QString kind = kindKey(clip);
    static const QHash<QString, QString> names{
        {QStringLiteral("video"), QStringLiteral("Video")},       {QStringLiteral("sequence"), QStringLiteral("Sequence")},
        {QStringLiteral("audio"), QStringLiteral("Audio")},       {QStringLiteral("title"), QStringLiteral("Title")},
        {QStringLiteral("solid"), QStringLiteral("Solid")},       {QStringLiteral("adjustment"), QStringLiteral("Adjustment")},
        {QStringLiteral("caption"), QStringLiteral("Caption")},
    };
    if (names.contains(kind)) return names.value(kind);
    return titleCase(clip.clip.value(QStringLiteral("fx")).toString(QStringLiteral("Generator")));
}

QString InspectorPanel::displayName(const Clip &clip) const {
    // Its own name; else its media's file name; else what generates it;
    // else where it is ("Clip on V1"). Never a stand-in such as "Untitled media".
    const QString name = clip.clip.value(QStringLiteral("name")).toString().trimmed();
    if (!name.isEmpty() && name != QStringLiteral("Untitled media")) return name;
    const QString path = clip.source.value(QStringLiteral("path")).toString();
    if (!path.isEmpty()) return QFileInfo(path).fileName();
    if (clip.clip.value(QStringLiteral("kind")).toString() == QStringLiteral("generator")) {
        const QString generator = m_effectDefs.value(clip.clip.value(QStringLiteral("fx")).toString()).value(QStringLiteral("name")).toString();
        return generator.isEmpty() ? kindName(clip) : generator;
    }
    return QStringLiteral("Clip on %1").arg(trackBadge(m_project, clip.track.value(QStringLiteral("id")).toString()).badge);
}

double InspectorPanel::clipTime(const Clip &clip) const {
    return std::clamp(m_playhead - clip.clip.value(QStringLiteral("timelineStart")).toDouble(), 0.0,
                      clip.clip.value(QStringLiteral("duration")).toDouble());
}

QString InspectorPanel::shapeKey() const {
    QStringList parts{QString::number(int(m_pane)), m_finish ? key("finish") : key("-"), QString::number(int(m_mode)),
                      QString::number(m_clips.size()), m_maskable ? key("m") : key("-"), m_pictureTracks.join(QLatin1Char(',')),
                      m_soundTracks.join(QLatin1Char(',')), m_silent ? key("silent") : key("")};
    if (m_pane == Pane::Info) {
        // Text only: built again whenever what it says changes.
        const QJsonObject info = m_clips.isEmpty() ? QJsonObject{}
                                                   : m_mediaInfo.value(m_clips.first().clip.value(QStringLiteral("sourceId")).toString()).toObject();
        parts << (m_clips.isEmpty() ? QString() : QString::fromUtf8(QJsonDocument(m_clips.first().clip).toJson(QJsonDocument::Compact)))
              << QString::fromUtf8(QJsonDocument(info).toJson(QJsonDocument::Compact))
              << QString::number(m_project.value(QStringLiteral("duration")).toDouble())
              << QString::number(m_project.value(QStringLiteral("fps")).toDouble());
        for (const Clip &clip : m_clips) parts << clip.track.value(QStringLiteral("role")).toString();
        return parts.join(QLatin1Char('|'));
    }
    if (m_mode == Mode::Sequence) {
        // The Format rows are plain text, built once per shape: their values
        // are part of it, so a new rate or size builds them again.
        const QSize canvas = resolvedCanvasSize(m_project, m_project.value(QStringLiteral("sources")).toArray());
        parts << QString::number(m_project.value(QStringLiteral("tracks")).toArray().size())
              << QString::number(m_project.value(QStringLiteral("markers")).toArray().size())
              << QString::number(fpsOf()) << QString::number(m_project.value(QStringLiteral("dropFrame")).toBool())
              << QString::number(m_project.value(QStringLiteral("sampleRate")).toInt())
              << QString::number(m_project.value(QStringLiteral("start")).toDouble())
              << QStringLiteral("%1x%2").arg(canvas.width()).arg(canvas.height());
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
    refreshFooter();
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
    m_targetSections.clear();
    m_rowSections.clear();
    m_info.clear();
    m_hint = nullptr;
    m_hintText = nullptr;
    m_wheels = nullptr;
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
    body->setContentsMargins(0, 0, 0, 0);
    body->setSpacing(0);
    const QString pane = m_pane == Pane::Video ? key("video") : m_pane == Pane::Audio ? key("audio") : key("info");
    m_kindForSections = pane + QLatin1Char(':') +
                        (m_mode == Mode::Sequence     ? QStringLiteral("sequence")
                         : m_mode == Mode::MixedKinds ? QStringLiteral("mixed")
                         : m_mode == Mode::Several    ? QStringLiteral("several-") + kindKey(m_clips.first())
                                                      : kindKey(m_clips.first()));
    if (m_pane == Pane::Info) {
        buildInfo(body);
    } else if (m_pane == Pane::Audio) {
        if (m_mode == Mode::Clip || m_mode == Mode::Several) buildAudioPane(body);
    } else if (m_mode == Mode::Sequence) {
        buildSequence(body);
    } else if (m_mode == Mode::MixedKinds) {
        // Clips of different kinds share their timing only.
        buildTimeSection(body);
    } else {
        const QString kind = kindKey(m_clips.first());
        if (m_finish && m_mode == Mode::Clip && kind != QStringLiteral("audio") && kind != QStringLiteral("caption")) buildWheels(body);
        if (kind == QStringLiteral("video") || kind == QStringLiteral("sequence")) buildVideo(body, false);
        else if (kind == QStringLiteral("audio")) buildAudio(body);
        else if (kind == QStringLiteral("title")) buildTitle(body);
        else if (kind == QStringLiteral("solid")) buildSolid(body);
        else if (kind == QStringLiteral("adjustment")) buildAdjustment(body);
        else if (kind == QStringLiteral("caption")) buildCaption(body);
        else buildGenerator(body);
    }
    if (m_pane == Pane::Video && m_mode == Mode::Clip) buildProposalHint();
    m_bodyCost = cost.elapsed();
    showBody();
}

void InspectorPanel::showBody() {
    m_contentLayout->insertWidget(m_contentLayout->indexOf(m_note) + 1, m_bodyWidget);
    m_bodyWidget->show();
    // Back where the body was for this kind, once it is laid out.
    const int offset = m_scrollByKind.value(m_kindForSections);
    m_scroll->viewport()->setUpdatesEnabled(false);
    QTimer::singleShot(0, this, [this, offset, shape = m_shape] {
        if (shape == m_shape) m_scroll->verticalScrollBar()->setValue(offset);
        m_scroll->viewport()->setUpdatesEnabled(true);
        updateFade();
    });
}

void InspectorPanel::parkBody() {
    if (!m_bodyWidget) return;
    if (m_shape.isEmpty() || m_bodyCost < kParkedBodyCost || m_pane == Pane::Info) {
        clearBody();
        return;
    }
    if (!m_parking) m_parking = new QWidget;
    m_contentLayout->removeWidget(m_bodyWidget);
    m_bodyWidget->hide();
    m_bodyWidget->setParent(m_parking);
    m_parked.insert(m_shape, ParkedBody{m_bodyWidget, m_kindForSections, m_rows, m_sections, m_refreshers, m_effectBoxes,
                                        m_timingFields, m_targetSections, m_rowSections, m_hint, m_hintText, m_wheels});
    m_parkedOrder.removeAll(m_shape);
    m_parkedOrder.append(m_shape);
    while (m_parkedOrder.size() > kParkedBodies) delete m_parked.take(m_parkedOrder.takeFirst()).widget;
    m_bodyWidget = nullptr;
    m_timingFields.clear();
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
    m_timingFields = parked.timingFields;
    m_targetSections = parked.targetSections;
    m_rowSections = parked.rowSections;
    m_hint = parked.hint;
    m_hintText = parked.hintText;
    m_wheels = parked.wheels;
    m_bodyCost = kParkedBodyCost;
    showBody();
    return true;
}

void InspectorPanel::forgetBodies() {
    m_shape.clear();
    for (const ParkedBody &parked : std::as_const(m_parked)) delete parked.widget;
    m_parked.clear();
    m_parkedOrder.clear();
}

void InspectorPanel::updateFade() {
    if (!m_fade) return;
    const QRect viewport = m_scroll->viewport()->geometry();
    m_fade->setGeometry(viewport.left(), viewport.bottom() + 1 - kFadeHeight, viewport.width(), kFadeHeight);
    QScrollBar *bar = m_scroll->verticalScrollBar();
    m_fade->setVisible(bar->maximum() > 0 && bar->value() < bar->maximum());
    m_fade->raise();
}

// ---------------------------------------------------------------- header

void InspectorPanel::buildHeader() {
    auto *layout = new QHBoxLayout(m_header);
    layout->setContentsMargins(12, 4, 8, 4);
    layout->setSpacing(8);
    auto *text = new QVBoxLayout;
    text->setContentsMargins(0, 0, 0, 0);
    text->setSpacing(0);
    m_name = new NameLabel;
    connect(m_name, &NameLabel::renamed, this, [this](const QString &name) {
        if (m_mode != Mode::Clip || m_pane != Pane::Video) return;
        emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("set")}, {QStringLiteral("clip"), m_clips.first().id},
                                       {QStringLiteral("name"), name}}});
    });
    text->addWidget(m_name);
    m_kind = styledLabel(QString(), tokens::monoSmall, tokens::ink3);
    m_kind->setObjectName(QStringLiteral("inspectorKind"));
    m_kind->setFixedHeight(16);
    m_kind->setMinimumWidth(0);
    m_kind->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Fixed);
    text->addWidget(m_kind);
    layout->addLayout(text, 1);
    m_more = new ui::IconButton(QStringLiteral("g-more"));
    m_more->setObjectName(QStringLiteral("inspectorMore"));
    m_more->setTip(QStringLiteral("Ask Kerf, reveal in project, reset"));
    m_moreMenu = new QMenu(this);
    m_moreMenu->setObjectName(QStringLiteral("inspectorMoreMenu"));
    ui::styleMenu(m_moreMenu);
    connect(m_more, &QAbstractButton::clicked, this, [this] {
        // Ask's key as it stands now: a keymap may have moved it.
        for (QAction *action : m_moreMenu->actions())
            if (action->objectName() == QStringLiteral("inspectorAsk")) ui::setMenuKey(action, askKey());
        ui::popupMenu(m_moreMenu, m_more);
    });
    layout->addWidget(m_more, 0, Qt::AlignTop);
}

void InspectorPanel::setAskAction(QAction *action) { m_askAction = action; }

QString InspectorPanel::askKey() const {
    return m_askAction ? m_askAction->shortcut().toString(QKeySequence::NativeText) : QString();
}

void InspectorPanel::refreshHeader() {
    const double fps = fpsOf();
    const bool drop = m_project.value(QStringLiteral("dropFrame")).toBool();
    const auto tc = [fps, drop](qint64 frames) { return frameTimecode(frames, fps, drop); };
    const auto setMeta = [this](const QString &meta) {
        m_kind->setProperty("full", meta);
        m_kind->setToolTip(meta);
        m_kind->setText(m_kind->fontMetrics().elidedText(meta, Qt::ElideRight, std::max(40, m_kind->width())));
    };
    m_moreMenu->clear();
    m_note->hide();
    m_name->setEditable(m_mode == Mode::Clip && m_pane == Pane::Video);
    const auto ask = [this](const QString &label, const QString &subject) {
        QAction *action = m_moreMenu->addAction(label, this, [this, subject] { askAbout(subject); });
        action->setObjectName(QStringLiteral("inspectorAsk"));
        ui::setMenuKey(action, askKey());
    };
    if (m_mode == Mode::Sequence) {
        QString name;
        for (const QJsonValue &sequence : m_project.value(QStringLiteral("sequences")).toArray())
            if (sequence.toObject().value(QStringLiteral("id")) == m_project.value(QStringLiteral("sequence")))
                name = sequence.toObject().value(QStringLiteral("name")).toString();
        if (name.isEmpty()) name = m_project.value(QStringLiteral("name")).toString(QStringLiteral("Sequence"));
        m_name->setName(name);
        const QSize size = resolvedCanvasSize(m_project, m_project.value(QStringLiteral("sources")).toArray());
        double duration = m_project.value(QStringLiteral("duration")).toDouble();
        if (!m_project.contains(QStringLiteral("duration")))
            for (const QJsonValue &track : m_project.value(QStringLiteral("tracks")).toArray())
                for (const QJsonValue &clip : track.toObject().value(QStringLiteral("clips")).toArray())
                    duration = std::max(duration, clip.toObject().value(QStringLiteral("timelineStart")).toDouble() +
                                                      clip.toObject().value(QStringLiteral("duration")).toDouble());
        setMeta(QStringLiteral("%1×%2 · %3 · %4").arg(size.width()).arg(size.height()).arg(rateText(fps), tc(std::llround(duration * fps))));
        m_note->setText(m_pane == Pane::Audio ? QStringLiteral("Select a clip to adjust its sound.")
                                              : m_pane == Pane::Video ? QStringLiteral("Select a clip to adjust it.") : QString());
        m_note->setVisible(!m_note->text().isEmpty());
        ask(QStringLiteral("Ask Kerf about the sequence"), QStringLiteral("the sequence"));
        return;
    }
    if (m_silent) {
        m_name->setName(m_selection.size() == 1 ? QStringLiteral("No sound") : QStringLiteral("%1 clips").arg(m_selection.size()));
        setMeta(QStringLiteral("No audio on the selection"));
        m_note->setText(m_selection.size() == 1 ? QStringLiteral("This clip carries no sound.") : QStringLiteral("These clips carry no sound."));
        m_note->show();
        return;
    }
    const Clip &first = m_clips.first();
    const auto badgeOf = [this](const QJsonObject &track) { return trackBadge(m_project, track.value(QStringLiteral("id")).toString()).badge; };
    if (m_mode == Mode::Clip) {
        m_name->setName(displayName(first));
        const qint64 start = qint64(first.clip.value(QStringLiteral("startFrame")).toDouble(std::round(first.clip.value(QStringLiteral("timelineStart")).toDouble() * fps)));
        const qint64 length = qint64(first.clip.value(QStringLiteral("durationFrames")).toDouble(std::round(first.clip.value(QStringLiteral("duration")).toDouble() * fps)));
        const QJsonObject info = m_mediaInfo.value(first.clip.value(QStringLiteral("sourceId")).toString()).toObject();
        if (m_pane == Pane::Audio) {
            // "A1 linked · 48 kHz · 24-bit · stereo"
            QStringList parts{badgeOf(first.track) + (first.linked.isEmpty() || first.track.value(QStringLiteral("type")).toString() != QStringLiteral("audio") ||
                                                              m_selection.contains(first.id)
                                                          ? QString()
                                                          : QStringLiteral(" linked"))};
            if (const QString rate = kiloHertz(first.source.value(QStringLiteral("sampleRate")).toDouble()); !rate.isEmpty()) parts << rate;
            if (const int bits = info.value(QStringLiteral("audioBitDepth")).toInt(); bits > 0) parts << QStringLiteral("%1-bit").arg(bits);
            if (const QString channels = channelsText(first.source.value(QStringLiteral("channels")).toInt()); !channels.isEmpty()) parts << channels;
            setMeta(parts.join(QStringLiteral(" · ")));
        } else if (m_pane == Pane::Info) {
            QStringList parts;
            if (const QString camera = info.value(QStringLiteral("camera")).toString(); !camera.isEmpty()) parts << camera;
            if (const QString codec = codecText(info); !codec.isEmpty()) parts << codec;
            if (parts.isEmpty()) parts << kindName(first);
            setMeta(parts.join(QStringLiteral(" · ")));
        } else {
            // "V1 · 00:00:08:12–00:00:13:05 · 4s18f"
            QString meta = QStringLiteral("%1 · %2–%3 · %4").arg(badgeOf(first.track), tc(start), tc(start + length), durationText(length, fps));
            if (!first.clip.value(QStringLiteral("enabled")).toBool(true)) meta += QStringLiteral(" · Off");
            setMeta(meta);
            if (kindKey(first) == QStringLiteral("adjustment")) {
                m_note->setText(QStringLiteral("Applies its effects to every layer beneath it."));
                m_note->show();
            }
        }
        ask(QStringLiteral("Ask Kerf about this clip"), QLatin1Char('@') + displayName(first));
        if (!first.source.isEmpty()) {
            QAction *reveal = m_moreMenu->addAction(QStringLiteral("Reveal in project"), this, [this] {
                emit revealRequested(m_clips.first().clip.value(QStringLiteral("sourceId")).toString());
            });
            reveal->setObjectName(QStringLiteral("inspectorReveal"));
        }
    } else {
        m_name->setName(QStringLiteral("%1 clips").arg(m_clips.size()));
        QMap<QString, int> kinds;
        QSet<QString> trackIds;
        for (const Clip &clip : m_clips) {
            kinds[kindName(clip)] += 1;
            trackIds.insert(badgeOf(clip.track));
        }
        if (kinds.size() == 1) {
            setMeta(kinds.firstKey() + (trackIds.size() == 1 ? QStringLiteral(" · ") + *trackIds.cbegin() : QString()));
        } else {
            QStringList parts;
            for (auto it = kinds.cbegin(); it != kinds.cend(); ++it) parts << QStringLiteral("%1 %2").arg(it.value()).arg(it.key());
            setMeta(parts.join(QStringLiteral(", ")));
        }
        if (m_mode == Mode::MixedKinds && m_pane == Pane::Video) {
            const bool sound = std::any_of(m_clips.cbegin(), m_clips.cend(), [this](const Clip &c) { return kindKey(c) == QStringLiteral("audio"); });
            m_note->setText(sound ? QStringLiteral("Video and audio clips have no properties in common.")
                                  : QStringLiteral("These clips have no properties in common."));
            m_note->show();
        }
        QStringList names;
        for (const Clip &clip : std::as_const(m_clips)) names << QLatin1Char('@') + displayName(clip);
        ask(QStringLiteral("Ask Kerf about these clips"), names.join(QLatin1Char(' ')));
    }
    if (m_pane == Pane::Info) return;
    m_moreMenu->addSeparator();
    QAction *reset = m_moreMenu->addAction(QStringLiteral("Reset all properties"), this, &InspectorPanel::resetAll);
    reset->setObjectName(QStringLiteral("inspectorResetAllItem"));
    reset->setShortcut(QKeySequence(Qt::CTRL | Qt::ALT | Qt::Key_R));
    reset->setShortcutVisibleInContextMenu(true);
}

void InspectorPanel::refreshFooter() {
    // The Video pane's footer. Saving needs one clip with effects, and the
    // bridge's look_save.
    m_footer->setVisible(m_pane == Pane::Video);
    const bool hasEffects = m_mode == Mode::Clip && !m_clips.first().clip.value(QStringLiteral("effects")).toArray().isEmpty();
    m_savePreset->setEnabled(m_presetSaving && hasEffects);
    m_savePreset->setTip(!m_presetSaving ? m_presetReason
                         : !hasEffects   ? QStringLiteral("Save effects preset: select a clip with effects")
                                         : QStringLiteral("Save this clip's effects as a preset"));
}

void InspectorPanel::timingCommitted(const QString &field, double value, double delta, bool relative) {
    const double fps = fpsOf();
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
    const QString id = m_clips.first().clip.value(QStringLiteral("effects")).toArray().at(effect).toObject().value(QStringLiteral("fx")).toString();
    for (const QJsonValue &def : m_effectDefs.value(id).value(QStringLiteral("params")).toArray())
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

int InspectorPanel::effectIndexOf(const Clip &clip, const QString &id) const {
    const QJsonArray effects = clip.clip.value(QStringLiteral("effects")).toArray();
    for (int index = 0; index < effects.size(); ++index)
        if (effects.at(index).toObject().value(QStringLiteral("fx")).toString() == id) return index;
    return -1;
}

bool InspectorPanel::targets(const Clip &clip, const Target &target) const {
    switch (target.kind) {
    case Target::Kind::Prop:
    case Target::Kind::Fx:
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
    case Target::Kind::Fx: {
        const int index = effectIndexOf(clip, target.fx);
        if (index < 0) return {};
        return clip.clip.value(QStringLiteral("effects")).toArray().at(index).toObject().value(QStringLiteral("params")).toObject().value(target.name);
    }
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
        if (keys.isEmpty()) return def.value(QStringLiteral("default"));
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
    case Target::Kind::Fx: {
        const QJsonArray ops = effectParamOps(clip, target.fx, QJsonObject{{target.name, value}});
        return ops.isEmpty() ? QJsonObject{} : ops.first().toObject();
    }
    case Target::Kind::Mask:
        return {{QStringLiteral("op"), QStringLiteral("mask-set")}, {QStringLiteral("clip"), clip.id}, {QStringLiteral("name"), target.mask},
                {QStringLiteral("params"), QJsonObject{{target.name, value}}}};
    }
    return {};
}

QJsonArray InspectorPanel::effectParamOps(const Clip &clip, const QString &id, const QJsonObject &params) const {
    const int index = effectIndexOf(clip, id);
    if (index >= 0)
        return {QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-set")}, {QStringLiteral("clip"), clip.id},
                            {QStringLiteral("index"), index}, {QStringLiteral("params"), params}}};
    // Not on the clip yet: the first edit adds it with the value, nulls
    // (defaults) left out.
    QJsonObject given;
    for (auto it = params.constBegin(); it != params.constEnd(); ++it)
        if (!it.value().isNull()) given.insert(it.key(), it.value());
    QJsonObject add{{QStringLiteral("op"), QStringLiteral("fx-add")}, {QStringLiteral("clip"), clip.id}, {QStringLiteral("fx"), id}};
    if (!given.isEmpty()) add.insert(QStringLiteral("params"), given);
    return {add};
}

void InspectorPanel::emitOps(const QJsonArray &ops) {
    if (!ops.isEmpty()) emit editRequested(ops);
}

void InspectorPanel::commitNumber(const Target &target, const QJsonObject &def, double value, double delta, bool relative) {
    QJsonArray ops;
    const bool several = m_clips.size() > 1;
    for (const Clip &clip : std::as_const(m_clips)) {
        if (!targets(clip, target)) continue;
        const NumberFormat format = formatFor(clip, def);
        const double current = displayOf(clip, target, def);
        // Several clips move alike from their own values; one takes the value.
        const double display = std::clamp(several && relative ? current + delta : value, format.min, format.max);
        if (formatNumber(format, display) == formatNumber(format, current)) continue;
        const double model = display / format.scale;
        if (isAnimated(clip, target)) {
            // Animated: the value becomes the key at the playhead (set, or
            // added there), as an editor's keyframed parameter does.
            const qint64 frame = playheadFrame(clip);
            if (frame < 0 || !keyframeRefusal(clip, target, def).isEmpty()) continue;
            QList<Key> keys = keysOf(clip, target);
            bool replaced = false;
            for (Key &k : keys)
                if (k.frame == frame) {
                    k.value = model;
                    replaced = true;
                }
            if (!replaced) {
                keys.append(Key{frame, model, QStringLiteral("linear")});
                std::sort(keys.begin(), keys.end(), [](const Key &a, const Key &b) { return a.frame < b.frame; });
            }
            ops.append(opFor(clip, target, keysJson(keys)));
            continue;
        }
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

// ------------------------------------------------------------- keyframes

double InspectorPanel::fpsOf() const {
    return m_project.value(QStringLiteral("fps")).toDouble(24.0);
}

qint64 InspectorPanel::playheadFrame(const Clip &clip) const {
    const double fps = fpsOf();
    const qint64 start = qint64(clip.clip.value(QStringLiteral("startFrame")).toDouble(std::round(clip.clip.value(QStringLiteral("timelineStart")).toDouble() * fps)));
    const qint64 length = qint64(clip.clip.value(QStringLiteral("durationFrames")).toDouble(std::round(clip.clip.value(QStringLiteral("duration")).toDouble() * fps)));
    const qint64 frame = std::llround(m_playhead * fps) - start;
    return frame >= 0 && frame < length ? frame : -1;
}

QList<InspectorPanel::Key> InspectorPanel::keysOf(const Clip &clip, const Target &target) const {
    QList<Key> keys;
    const QJsonValue raw = rawValue(clip, target);
    if (!raw.isArray()) return keys;
    const double fps = fpsOf();
    for (const QJsonValue &value : raw.toArray()) {
        const QJsonObject k = value.toObject();
        // The timeline gives each key its frame next to its time.
        const qint64 frame = k.contains(QStringLiteral("frame")) ? qint64(k.value(QStringLiteral("frame")).toDouble())
                                                                 : std::llround(k.value(QStringLiteral("at")).toDouble() * fps);
        keys.append(Key{frame, k.value(QStringLiteral("value")), k.value(QStringLiteral("interp")).toString(QStringLiteral("linear"))});
    }
    return keys;
}

QJsonValue InspectorPanel::keysJson(const QList<Key> &keys) {
    QJsonArray list;
    for (const Key &k : keys)
        list.append(QJsonObject{{QStringLiteral("at"), k.frame}, {QStringLiteral("value"), k.value}, {QStringLiteral("interp"), k.interp}});
    return QJsonObject{{QStringLiteral("keys"), list}};
}

QString InspectorPanel::keyframeRefusal(const Clip &clip, const Target &target, const QJsonObject &def) const {
    if (!def.value(QStringLiteral("animatable")).toBool(true)) return QStringLiteral("This value cannot be animated");
    // kerfd keys clip gain only on sound: playback and renders follow no
    // other sound keyframes (E1-RUNTIME).
    const QJsonObject track = target.kind == Target::Kind::Linked ? clip.linkedTrack : clip.track;
    if (track.value(QStringLiteral("type")).toString() == QStringLiteral("audio")) {
        const bool gain = (target.kind == Target::Kind::Prop || target.kind == Target::Kind::Linked) && target.name == QStringLiteral("gain");
        if (!gain) return QStringLiteral("Playback and renders follow keyframes on gain only");
    }
    if (playheadFrame(clip) < 0) return QStringLiteral("Move the playhead over the clip to set a keyframe");
    return {};
}

void InspectorPanel::bindKeyframe(int rowIndex, const Target &target, const QJsonObject &def) {
    if (def.value(QStringLiteral("type")).toString() != QStringLiteral("number")) return;
    if (def.contains(QStringLiteral("animatable")) && !def.value(QStringLiteral("animatable")).toBool()) return;
    RowEntry &entry = m_rows[rowIndex];
    const bool first = entry.keyed.isEmpty();
    entry.keyed.append(Keyed{target, def});
    KeyframeButton *button = entry.row->keyframeButton();
    button->show();
    if (first) connect(button, &QAbstractButton::clicked, this, [this, rowIndex] { toggleKeyframe(rowIndex); });
}

void InspectorPanel::refreshKeyframe(int rowIndex) {
    const RowEntry &entry = m_rows.at(rowIndex);
    KeyframeButton *button = entry.row->keyframeButton();
    QString refusal;
    bool has = false;
    bool animated = false;
    bool any = false;
    for (const Clip &clip : std::as_const(m_clips))
        for (const Keyed &keyed : entry.keyed) {
            if (!targets(clip, keyed.target)) continue;
            if (!any) refusal = keyframeRefusal(clip, keyed.target, keyed.def);
            any = true;
            const qint64 frame = playheadFrame(clip);
            const QList<Key> keys = keysOf(clip, keyed.target);
            animated |= !keys.isEmpty();
            for (const Key &k : keys) has |= frame >= 0 && k.frame == frame;
        }
    button->setEnabled(any && refusal.isEmpty());
    button->setHas(has && refusal.isEmpty());
    button->setAnimated(animated);
    ui::setTip(button, !refusal.isEmpty() ? refusal : has ? QStringLiteral("Keyframe at playhead") : QStringLiteral("Add keyframe"));
}

void InspectorPanel::toggleKeyframe(int rowIndex) {
    const RowEntry &entry = m_rows.at(rowIndex);
    const bool removing = entry.row->keyframeButton()->has();
    QJsonArray ops;
    for (const Clip &clip : std::as_const(m_clips))
        for (const Keyed &keyed : entry.keyed) {
            if (!targets(clip, keyed.target) || !keyframeRefusal(clip, keyed.target, keyed.def).isEmpty()) continue;
            const qint64 frame = playheadFrame(clip);
            QList<Key> keys = keysOf(clip, keyed.target);
            const QJsonValue now = valueOf(clip, keyed.target, keyed.def);
            if (removing) {
                const qsizetype before = keys.size();
                keys.erase(std::remove_if(keys.begin(), keys.end(), [frame](const Key &k) { return k.frame == frame; }), keys.end());
                if (keys.size() == before) continue;
                if (keys.isEmpty()) {
                    // The last key gone: the value it had stays, still.
                    const bool isDefault = sameJson(now, keyed.def.value(QStringLiteral("default")));
                    ops.append(opFor(clip, keyed.target, isDefault ? QJsonValue() : now));
                } else {
                    ops.append(opFor(clip, keyed.target, keysJson(keys)));
                }
            } else {
                keys.append(Key{frame, now, QStringLiteral("linear")});
                std::sort(keys.begin(), keys.end(), [](const Key &a, const Key &b) { return a.frame < b.frame; });
                ops.append(opFor(clip, keyed.target, keysJson(keys)));
            }
        }
    emitOps(ops);
}

// ------------------------------------------------------- rows and groups

int InspectorPanel::addSection(QVBoxLayout *into, const QString &id, const QString &title, bool openByDefault) {
    auto *widget = new InspectorSection(id, title);
    into->addWidget(widget);
    widget->setOpen(savedOpen(id, openByDefault));
    connect(widget, &InspectorSection::toggled, this, [this, id](bool open, Qt::KeyboardModifiers modifiers) {
        setOpenSections(id, open, modifiers);
    });
    connect(widget, &InspectorSection::foldChanged, this, &InspectorPanel::updateFade);
    const int index = int(m_sections.size());
    widget->setResetHandler([this, index] {
        QJsonArray ops;
        for (const int rowIndex : m_sections.at(index).rows)
            for (const Binding &binding : m_rows.at(rowIndex).values)
                if (binding.resetOps)
                    for (const QJsonValue &op : binding.resetOps()) ops.append(op);
        emitOps(ops);
    });
    m_sections.append(SectionEntry{widget, {}, {}, {}, {}});
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
// Each group's fold as last left, by id, per pane and kind of clip.
QString foldsKey(const QString &kind) {
    return QStringLiteral("editor/inspector/folds/%1").arg(kind);
}
} // namespace

bool InspectorPanel::savedOpen(const QString &id, bool openByDefault) const {
    const QVariantMap folds = QSettings().value(foldsKey(m_kindForSections)).toMap();
    if (const auto found = folds.constFind(id); found != folds.cend()) return found->toBool();
    return openByDefault;
}

void InspectorPanel::saveSections() {
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
    m_rows.append(RowEntry{widget, {}, {}});
    if (section >= 0) {
        m_sections[section].rows.append(index);
        m_rowSections.insert(index, section);
    }
    return index;
}

ScrubField *InspectorPanel::bindNumber(int rowIndex, QHBoxLayout *into, const Target &target, const QJsonObject &def,
                                       bool withSlider, bool primaryField) {
    auto *field = newScrubField();
    InspectorSlider *slider = nullptr;
    InspectorRow *r = row(rowIndex);
    if (withSlider) {
        // The slider fills the control column; its value sits in the value column.
        slider = new InspectorSlider;
        slider->setObjectName(QStringLiteral("slider_") + target.name);
        into->addWidget(slider, 1);
        r->addValue(field);
        r->setWide(false);
    } else {
        // A value (or an X/Y pair) at the right end of the control and
        // value columns; the label runs on as far as they leave.
        if (into->count() == 0) into->addStretch(1);
        into->addWidget(field);
        r->setWide(true);
    }
    if (primaryField) r->label()->setField(field);
    const QString label = presentationLabel(def);
    Binding binding;
    binding.refresh = [this, field, slider, target, def, r] {
        QList<double> values;
        QStringList shown;
        bool animated = false;
        bool still = false;
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
                // An animated value edits its key at the playhead: outside
                // the clip there is none to set.
                still |= playheadFrame(clip) < 0 || !keyframeRefusal(clip, target, def).isEmpty();
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
        field->setReadOnly(animated && still);
        if (animated) r->setAnimated(true);
        if (slider) {
            double low = 0.0, high = 1.0;
            sliderTravel(target.name, format, &low, &high);
            slider->setRange(low, high);
            slider->setGainTravel(format.kind == NumberFormat::Kind::Gain);
            const bool bipolar = low < 0.0 && high > 0.0;
            slider->setOrigin(bipolar ? 0.0 : low);
            slider->setDetent(fallback);
            slider->setValue(values.first());
            slider->setMixed(mixed);
            slider->setEnabled(!(animated && still));
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
            NumberFormat shown = f;
            if (shown.unit == QStringLiteral("px")) shown.decimals = 0;
            return QStringLiteral("%1 %2").arg(name, formatWithUnit(shown, display));
        }
        return QString();
    };
    binding.resetOps = [this, target, def] { return resetOpsFor(target, def); };
    m_rows[rowIndex].values.append(binding);
    m_targetSections.insert(targetKey(target), m_rowSections.value(rowIndex, -1));
    bindKeyframe(rowIndex, target, def);
    connect(field, &ScrubField::committed, this, [this, target, def](double value, double delta, bool relative) {
        commitNumber(target, def, value, delta, relative);
    });
    if (slider) {
        connect(slider, &InspectorSlider::moved, field, [field](double value) { field->setValue(value); });
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
    InspectorRow *r = row(rowIndex);
    QWidget *editor = nullptr;
    std::function<void(const QJsonValue &)> show;
    if (type == QStringLiteral("enum") || type == QStringLiteral("track") || type == QStringLiteral("sound") ||
        type == QStringLiteral("mask")) {
        // A ghost pop-up at the row's right end (.gv).
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
            combo->updateGeometry();
        };
        if (into->count() == 0) into->addStretch(1);
        into->addWidget(combo);
        r->setWide(true);
        editor = combo;
    } else if (type == QStringLiteral("bool")) {
        auto *toggle = new InspectorSwitch;
        connect(toggle, &QAbstractButton::clicked, this, [this, target, def](bool on) { commitValue(target, def, on); });
        show = [toggle](const QJsonValue &value) {
            const QSignalBlocker blocker(toggle);
            toggle->setChecked(value.toBool());
        };
        if (into->count() == 0) into->addStretch(1);
        into->addWidget(toggle, 0, Qt::AlignVCenter);
        r->setWide(true);
        toggle->setObjectName(QStringLiteral("value_") + target.name);
        editor = toggle;
    } else if (type == QStringLiteral("color")) {
        auto *holder = new QWidget;
        auto *holderLayout = new QHBoxLayout(holder);
        holderLayout->setContentsMargins(0, 0, 0, 0);
        holderLayout->setSpacing(8);
        auto *hex = styledLabel(QString(), tokens::timecode, tokens::ink);
        hex->setObjectName(QStringLiteral("colorHex"));
        auto *well = new QPushButton;
        well->setProperty("colorWell", true);
        well->setFlat(true);
        well->setCursor(Qt::PointingHandCursor);
        well->setFixedSize(32, 16);
        well->setIconSize(QSize(32, 16));
        well->setStyleSheet(QStringLiteral("QPushButton { background: transparent; border: none; padding: 0; }"));
        holderLayout->addStretch(1);
        holderLayout->addWidget(hex);
        holderLayout->addWidget(well);
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
    } else {
        // Text, or a file: a line on the raised fill, the file's name only.
        auto *line = new QLineEdit;
        line->setProperty("inspector", true);
        line->setFont(tokens::font(tokens::control));
        line->setFixedHeight(tokens::height::h24 - 4);
        line->setStyleSheet(QStringLiteral("QLineEdit { background: %1; color: %2; border: 1px solid transparent; border-radius: 4px; padding: 0 6px; }"
                                           "QLineEdit:focus { border-color: %3; }"
                                           "QLineEdit:disabled { color: %4; }")
                                .arg(css(tokens::raised), css(tokens::ink), css(tokens::accent), css(tokens::dis)));
        into->addWidget(line, 1);
        if (type == QStringLiteral("file")) {
            QStringList patterns;
            for (const QJsonValue &extension : def.value(QStringLiteral("extensions")).toArray())
                patterns << QStringLiteral("*.%1").arg(extension.toString());
            const QString filter = QStringLiteral("%1 (%2)").arg(label, patterns.join(QLatin1Char(' ')));
            QAction *choose = line->addAction(icons::icon(QStringLiteral("g-import"), {}, 14), QLineEdit::TrailingPosition);
            choose->setObjectName(QStringLiteral("chooseFile"));
            choose->setToolTip(QStringLiteral("Choose a file"));
            connect(choose, &QAction::triggered, this, [this, line, label, filter, target, def] {
                const QString path = QFileDialog::getOpenFileName(this, label, line->property("path").toString(), filter);
                if (!path.isEmpty()) commitValue(target, def, path);
            });
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
        } else {
            connect(line, &QLineEdit::editingFinished, this, [this, line, target, def] { commitValue(target, def, line->text()); });
            show = [line](const QJsonValue &value) {
                if (line->hasFocus()) return;
                const QSignalBlocker blocker(line);
                line->setText(value.toString());
            };
        }
        editor = line;
    }
    editor->setToolTip(def.value(QStringLiteral("doc")).toString().trimmed());
    Binding binding;
    binding.refresh = [this, show, target, def, editor, r] {
        bool animated = false;
        for (const Clip &clip : std::as_const(m_clips)) {
            if (!targets(clip, target)) continue;
            show(valueOf(clip, target, def));
            animated = isAnimated(clip, target);
            break;
        }
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
            const QString kind = def.value(QStringLiteral("type")).toString();
            const QString text = kind == QStringLiteral("enum") ? titleCase(value.toString())
                                 : kind == QStringLiteral("file") && !QFileInfo(value.toString()).fileName().isEmpty() ? QFileInfo(value.toString()).fileName()
                                                                                                                       : value.toString();
            return QStringLiteral("%1 %2").arg(label, text);
        }
        return QString();
    };
    binding.resetOps = [this, target, def] { return resetOpsFor(target, def); };
    m_rows[rowIndex].values.append(binding);
    m_targetSections.insert(targetKey(target), m_rowSections.value(rowIndex, -1));
    return editor;
}

void InspectorPanel::refreshValues() {
    for (const auto &refresher : std::as_const(m_refreshers)) refresher();
    for (int index = 0; index < m_rows.size(); ++index) {
        const RowEntry &entry = m_rows.at(index);
        entry.row->setAnimated(false);
        bool changed = false;
        for (const Binding &binding : entry.values) {
            if (binding.refresh) binding.refresh();
            if (binding.changed) changed |= binding.changed();
        }
        entry.row->setChanged(changed);
        if (!entry.keyed.isEmpty()) refreshKeyframe(index);
    }
    for (const SectionEntry &entry : std::as_const(m_sections)) {
        QStringList pieces;
        bool changed = false;
        for (const int rowIndex : entry.rows) {
            // A row out of the way says nothing of a value that does nothing yet.
            if (m_rows.at(rowIndex).row->isHidden()) continue;
            for (const Binding &binding : m_rows.at(rowIndex).values)
                if (binding.summary)
                    if (const QString piece = binding.summary(); !piece.isEmpty()) pieces << piece;
            changed |= m_rows.at(rowIndex).row->isChanged();
        }
        if (entry.items) entry.items(entry.section);
        else entry.section->setSummary(entry.summary ? entry.summary() : pieces.join(QStringLiteral(" · ")));
        entry.section->setChanged(changed);
        if (entry.enable) entry.enable();
    }
    placeProposalHint();
}

// ----------------------------------------------------------------- kinds

void InspectorPanel::buildTransformRows(QVBoxLayout *into, int section) {
    if (m_clips.isEmpty()) return;
    const Clip &first = m_clips.first();
    const auto def = [this, &first](const char *name) { return defNamed(first, QString::fromLatin1(name)); };
    const auto prop = [](const char *name) { return Target{Target::Kind::Prop, QString::fromLatin1(name)}; };
    // Position X / Y, Rotation, Scale (all), Scale X, Scale Y, Anchor X / Y (spec 5.1).
    if (!def("x").isEmpty() && !def("y").isEmpty()) {
        const int r = addRow(into, QStringLiteral("Position"), section);
        bindNumber(r, row(r)->controls(), prop("x"), def("x"))->setObjectName(QStringLiteral("prop_x"));
        bindNumber(r, row(r)->controls(), prop("y"), def("y"), false, false)->setObjectName(QStringLiteral("prop_y"));
    }
    if (!def("rotation").isEmpty()) {
        // Bipolar about 0°, as the knob it replaces turned either way.
        const int r = addRow(into, QStringLiteral("Rotation"), section);
        bindNumber(r, row(r)->controls(), prop("rotation"), def("rotation"), true)->setObjectName(QStringLiteral("prop_rotation"));
    }
    const struct {
        const char *name;
        const char *label;
    } scales[] = {{"scale", "Scale (all)"}, {"scale_x", "Scale X"}, {"scale_y", "Scale Y"}};
    for (const auto &scale : scales) {
        if (def(scale.name).isEmpty()) continue;
        const int r = addRow(into, QString::fromUtf8(scale.label), section);
        ScrubField *field = bindNumber(r, row(r)->controls(), prop(scale.name), def(scale.name), true);
        field->setObjectName(QStringLiteral("prop_") + QString::fromLatin1(scale.name));
        field->setProperty("noPrefix", true);
    }
    if (!def("anchor_x").isEmpty() && !def("anchor_y").isEmpty()) {
        const int r = addRow(into, QStringLiteral("Anchor"), section);
        bindNumber(r, row(r)->controls(), prop("anchor_x"), def("anchor_x"))->setObjectName(QStringLiteral("prop_anchor_x"));
        bindNumber(r, row(r)->controls(), prop("anchor_y"), def("anchor_y"), false, false)->setObjectName(QStringLiteral("prop_anchor_y"));
    }
    buildDepthRows(into, section);
}

// A clip in depth (kerf-compose `three_d.rs`). A camera and a light stand at a `z` whatever else is
// on, and a null or an adjustment layer has no picture to place. Anything else goes into depth with
// `three_d` and has these rows from then on; a bevel cuts a solid's edges, so it waits for a `depth`,
// and its profile for a bevel.
void InspectorPanel::buildDepthRows(QVBoxLayout *into, int section) {
    const Clip &first = m_clips.first();
    const QString fx = first.clip.value(QStringLiteral("kind")).toString() == QStringLiteral("generator") ? first.clip.value(QStringLiteral("fx")).toString() : QString();
    if (fx == QStringLiteral("null") || fx == QStringLiteral("adjustment")) return;
    const auto number = [this, into, section, &first](const QString &name, bool withSlider) -> InspectorRow * {
        const QJsonObject def = defNamed(first, name);
        if (def.isEmpty()) return nullptr;
        const int r = addRow(into, presentationLabel(def), section);
        bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, name}, def, withSlider)->setObjectName(QStringLiteral("prop_") + name);
        return row(r);
    };
    if (fx == QStringLiteral("camera") || fx == QStringLiteral("light")) {
        number(QStringLiteral("z"), false);
        return;
    }
    const QJsonObject threeD = defNamed(first, QStringLiteral("three_d"));
    if (threeD.isEmpty()) return;
    const int toggle = addRow(into, presentationLabel(threeD), section);
    bindValue(toggle, row(toggle)->controls(), Target{Target::Kind::Prop, QStringLiteral("three_d")}, threeD)->setObjectName(QStringLiteral("prop_three_d"));
    // Bipolar about 0°, as Rotation is; the distances run too far for a slider to say much.
    const QList<InspectorRow *> inDepth{number(QStringLiteral("z"), false), number(QStringLiteral("rotate_x"), true),
                                        number(QStringLiteral("rotate_y"), true), number(QStringLiteral("depth"), false)};
    InspectorRow *bevel = number(QStringLiteral("bevel"), false);
    InspectorRow *profile = nullptr;
    if (const QJsonObject def = defNamed(first, QStringLiteral("bevel_profile")); !def.isEmpty()) {
        const int r = addRow(into, presentationLabel(def), section);
        bindValue(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("bevel_profile")}, def)->setObjectName(QStringLiteral("prop_bevel_profile"));
        profile = row(r);
    }
    m_refreshers.append([this, inDepth, bevel, profile] {
        const Target depth{Target::Kind::Prop, QStringLiteral("depth")};
        const Target cut{Target::Kind::Prop, QStringLiteral("bevel")};
        // Above 0 at some moment, so one that grows as it goes counts.
        const auto ever = [](const QJsonValue &raw) {
            const QJsonArray keys = raw.toArray();
            return raw.toDouble() > 0.0 || std::any_of(keys.cbegin(), keys.cend(), [](const QJsonValue &key) { return key.toObject().value(QStringLiteral("value")).toDouble() > 0.0; });
        };
        bool placed = false;
        bool solid = false;
        bool cutting = false;
        for (const Clip &clip : std::as_const(m_clips)) {
            if (!clip.clip.value(QStringLiteral("props")).toObject().value(QStringLiteral("three_d")).toBool()) continue;
            placed = true;
            if (!ever(rawValue(clip, depth))) continue;
            solid = true;
            cutting |= ever(rawValue(clip, cut));
        }
        for (InspectorRow *entry : inDepth)
            if (entry) entry->setVisible(placed);
        if (bevel) bevel->setVisible(solid);
        // With no bevel, there is no cut for a profile to shape.
        if (profile) profile->setVisible(cutting);
    });
}

// Fit and Fill (the scale that shows the whole picture, or fills the frame,
// centred): a pop-up at the Transform group's header; "Custom" when the clip
// is neither.
void InspectorPanel::buildFitFill(int section) {
    if (m_clips.isEmpty() || defNamed(m_clips.first(), QStringLiteral("scale")).isEmpty()) return;
    auto *fit = popup(QStringLiteral("fitFill"));
    fit->addItem(QStringLiteral("Fit"), QStringLiteral("fit"));
    fit->addItem(QStringLiteral("Fill"), QStringLiteral("fill"));
    fit->setPlaceholderText(QStringLiteral("Custom"));
    fit->setToolTip(QStringLiteral("Fit: the whole picture in the frame. Fill: the frame filled, edges cropped"));
    m_sections.at(section).section->addTrailing(fit);
    const auto fillScale = [this](const Clip &clip) {
        const double w = clip.source.value(QStringLiteral("width")).toDouble();
        const double h = clip.source.value(QStringLiteral("height")).toDouble();
        const QSize canvas = resolvedCanvasSize(m_project, m_project.value(QStringLiteral("sources")).toArray());
        if (w <= 0 || h <= 0 || canvas.isEmpty()) return 1.0;
        const double media = w / h;
        const double frame = double(canvas.width()) / canvas.height();
        return std::max(media / frame, frame / media);
    };
    m_refreshers.append([this, fit, fillScale] {
        QSet<int> choices;
        for (const Clip &clip : std::as_const(m_clips)) {
            const QJsonObject props = clip.clip.value(QStringLiteral("props")).toObject();
            const double scale = props.value(QStringLiteral("scale")).toDouble(1.0);
            const bool centred = std::abs(props.value(QStringLiteral("x")).toDouble()) < 1e-6 && std::abs(props.value(QStringLiteral("y")).toDouble()) < 1e-6;
            const double fill = fillScale(clip);
            choices.insert(!centred ? -1 : std::abs(scale - 1.0) < 1e-3 ? 0 : std::abs(scale - fill) < 1e-3 * fill ? 1 : -1);
        }
        const QSignalBlocker blocker(fit);
        fit->setCurrentIndex(choices.size() == 1 ? *choices.cbegin() : -1);
        fit->updateGeometry();
    });
    connect(fit, &QComboBox::activated, this, [this, fillScale](int index) {
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

void InspectorPanel::buildCompositingRows(QVBoxLayout *into, int section) {
    if (m_clips.isEmpty()) return;
    const QJsonObject opacityDef = defNamed(m_clips.first(), QStringLiteral("opacity"));
    const QJsonObject blendDef = defNamed(m_clips.first(), QStringLiteral("blend"));
    if (!blendDef.isEmpty()) {
        const int r = addRow(into, QStringLiteral("Blend mode"), section);
        bindValue(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("blend")}, blendDef)->setObjectName(QStringLiteral("prop_blend"));
    }
    if (!opacityDef.isEmpty()) {
        const int r = addRow(into, QStringLiteral("Opacity"), section);
        bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("opacity")}, opacityDef, true)
            ->setObjectName(QStringLiteral("prop_opacity"));
    }
}

void InspectorPanel::buildTimeSection(QVBoxLayout *into) {
    if (m_clips.isEmpty()) return;
    // The clip's timing, speed and source range, folded at first.
    const int section = addSection(into, QStringLiteral("time"), QStringLiteral("Time"));
    buildTiming(sectionBody(section), section);
    if (std::all_of(m_clips.cbegin(), m_clips.cend(), [](const Clip &c) { return c.clip.contains(QStringLiteral("speed")); }))
        buildSpeedRow(sectionBody(section), section);
    buildSourceRows(sectionBody(section), section);
}

void InspectorPanel::buildTiming(QVBoxLayout *into, int section) {
    const double fps = fpsOf();
    const bool drop = m_project.value(QStringLiteral("dropFrame")).toBool();
    const struct {
        const char *field;
        const char *label;
    } columns[] = {{"start", "Start"}, {"end", "End"}, {"duration", "Duration"}};
    for (const auto &column : columns) {
        const QString edge = QString::fromLatin1(column.field);
        const int r = addRow(into, QString::fromLatin1(column.label), section);
        auto *field = newScrubField();
        field->setObjectName(QStringLiteral("timing_%1").arg(edge));
        field->setAccessibleName(QString::fromLatin1(column.label));
        NumberFormat format;
        format.kind = NumberFormat::Kind::Timecode;
        format.fps = fps;
        format.dropFrame = drop;
        format.min = edge == QStringLiteral("duration") ? 1.0 : 0.0;
        format.max = 1e9;
        format.step = 1.0;
        format.bigStep = 10.0;
        format.smallStep = 1.0;
        field->setFormat(format);
        row(r)->controls()->addStretch(1);
        row(r)->controls()->addWidget(field);
        row(r)->setWide(true);
        row(r)->label()->setField(field);
        connect(field, &ScrubField::committed, this, [this, edge](double value, double delta, bool relative) {
            timingCommitted(edge, value, delta, relative);
        });
        m_timingFields << field;
    }
    m_refreshers.append([this, fps] {
        if (m_timingFields.size() != 3 || m_clips.isEmpty()) return;
        qint64 start = std::numeric_limits<qint64>::max();
        qint64 end = 0;
        QSet<qint64> durations;
        for (const Clip &clip : std::as_const(m_clips)) {
            const qint64 s = qint64(clip.clip.value(QStringLiteral("startFrame")).toDouble(std::round(clip.clip.value(QStringLiteral("timelineStart")).toDouble() * fps)));
            const qint64 d = qint64(clip.clip.value(QStringLiteral("durationFrames")).toDouble(std::round(clip.clip.value(QStringLiteral("duration")).toDouble() * fps)));
            start = std::min(start, s);
            end = std::max(end, s + d);
            durations.insert(d);
        }
        m_timingFields.at(0)->setValue(double(start));
        m_timingFields.at(1)->setValue(double(end));
        m_timingFields.at(2)->setValue(double(*durations.cbegin()));
        m_timingFields.at(2)->setMixed(durations.size() > 1);
    });
}

void InspectorPanel::buildSpeedRow(QVBoxLayout *into, int section) {
    const int r = addRow(into, QStringLiteral("Speed"), section);
    auto *field = newScrubField();
    field->setObjectName(QStringLiteral("speed"));
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
    row(r)->setWide(true);
    auto *reverse = new GlyphButton(GlyphButton::Glyph::Reverse);
    reverse->setObjectName(QStringLiteral("speedReverse"));
    reverse->setCheckable(true);
    ui::setTip(reverse, QStringLiteral("Play backwards (−100%)"));
    reverse->setAccessibleName(QStringLiteral("Play backwards"));
    auto *presets = new GlyphButton(GlyphButton::Glyph::Menu);
    presets->setObjectName(QStringLiteral("speedPresets"));
    ui::setTip(presets, QStringLiteral("Speed presets"));
    auto *menu = new QMenu(presets);
    menu->setObjectName(QStringLiteral("speedPresetsMenu"));
    ui::styleMenu(menu);
    row(r)->controls()->addStretch(1);
    row(r)->controls()->addWidget(reverse);
    row(r)->controls()->addWidget(presets);
    row(r)->controls()->addWidget(field);
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
    connect(presets, &QAbstractButton::clicked, this, [presets, menu] { ui::popupMenu(menu, presets); });
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

void InspectorPanel::buildSourceRows(QVBoxLayout *into, int section) {
    if (m_clips.size() != 1 || m_clips.first().source.isEmpty() || !m_clips.first().clip.contains(QStringLiteral("in"))) return;
    const QJsonObject source = m_clips.first().source;
    const double fps = source.value(QStringLiteral("fps")).toDouble(fpsOf());
    // Source In and Out on the media's own rate; both slip the clip.
    NumberFormat tc;
    tc.kind = NumberFormat::Kind::Timecode;
    tc.fps = fps;
    tc.min = 0.0;
    tc.max = 1e9;
    tc.step = 1.0;
    tc.bigStep = 10.0;
    tc.smallStep = 1.0;
    for (const char *edge : {"in", "out"}) {
        const QString name = QString::fromLatin1(edge);
        const int r = addRow(into, name == QStringLiteral("in") ? QStringLiteral("Source in") : QStringLiteral("Source out"), section);
        auto *field = newScrubField();
        field->setObjectName(QStringLiteral("source_%1").arg(name));
        field->setFormat(tc);
        row(r)->controls()->addStretch(1);
        row(r)->controls()->addWidget(field);
        row(r)->setWide(true);
        row(r)->label()->setField(field);
        m_refreshers.append([this, field, name, fps] { field->setValue(std::round(m_clips.first().clip.value(name).toDouble() * fps)); });
        connect(field, &ScrubField::committed, this, [this, name, fps](double value, double, bool) {
            const Clip &clip = m_clips.first();
            const double speed = std::max(0.01, clip.clip.value(QStringLiteral("speed")).toDouble(1.0));
            const double seconds = value / fps - clip.clip.value(name).toDouble();
            const qint64 frames = std::llround(seconds / speed * fpsOf());
            if (frames == 0) return;
            emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("slip")}, {QStringLiteral("clip"), clip.id},
                                           {QStringLiteral("frames"), frames}}});
        });
    }
}

void InspectorPanel::buildCropSection(QVBoxLayout *into) {
    const Clip &first = m_clips.first();
    const QJsonObject l = defNamed(first, QStringLiteral("crop_l")), rr = defNamed(first, QStringLiteral("crop_r"));
    const QJsonObject t = defNamed(first, QStringLiteral("crop_t")), b = defNamed(first, QStringLiteral("crop_b"));
    if (l.isEmpty() || rr.isEmpty() || t.isEmpty() || b.isEmpty()) return;
    const int section = addSection(into, QStringLiteral("crop"), QStringLiteral("Crop"), true);
    m_sections.at(section).section->setEnable(InspectorSection::Enable::Always, QStringLiteral("Crop is always applied"));
    // Type: the engine crops by trimming the edges; it has no other mode.
    {
        const int r = addRow(sectionBody(section), QStringLiteral("Type"), section);
        auto *type = popup(QStringLiteral("cropType"));
        type->addItem(QStringLiteral("Trim"), QStringLiteral("trim"));
        type->setToolTip(QStringLiteral("Kerf crops by trimming the edges"));
        row(r)->controls()->addStretch(1);
        row(r)->controls()->addWidget(type);
        row(r)->setWide(true);
    }
    const auto field = [this](int r, const char *name, const QJsonObject &def, bool primary) {
        ScrubField *f = bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, QString::fromLatin1(name)}, def, false, primary);
        f->setObjectName(QStringLiteral("prop_") + QString::fromLatin1(name));
        f->setProperty("noPrefix", true);
    };
    const int across = addRow(sectionBody(section), QStringLiteral("Left / right"), section);
    field(across, "crop_l", l, true);
    field(across, "crop_r", rr, false);
    const int down = addRow(sectionBody(section), QStringLiteral("Top / bottom"), section);
    field(down, "crop_t", t, true);
    field(down, "crop_b", b, false);
}

void InspectorPanel::buildEffectGroup(QVBoxLayout *into, const QString &id, const QString &title, const QString &effect) {
    // A group of one effect (Distort, Stabilization, Channel EQ): its box is
    // the effect's enabled flag, unchecked while the clip has none; checking
    // it (or editing a row) adds it.
    const QJsonObject def = m_effectDefs.value(effect);
    if (def.isEmpty() || m_clips.size() != 1) return;
    const int section = addSection(into, id, title, false);
    InspectorSection *group = m_sections.at(section).section;
    group->setObjectName(QStringLiteral("group_") + id);
    for (const QJsonValue &value : def.value(QStringLiteral("params")).toArray()) {
        const QJsonObject param = value.toObject();
        const QString name = param.value(QStringLiteral("name")).toString();
        const Target target{Target::Kind::Fx, name, -1, QString(), effect};
        const int r = addRow(sectionBody(section), withoutSharedWord(presentationLabel(param), title), section);
        if (param.value(QStringLiteral("type")).toString() == QStringLiteral("number")) {
            const bool bounded = param.contains(QStringLiteral("min")) && param.contains(QStringLiteral("max"));
            bindNumber(r, row(r)->controls(), target, param, bounded)->setObjectName(QStringLiteral("fxParam_") + name);
        } else {
            bindValue(r, row(r)->controls(), target, param)->setObjectName(QStringLiteral("fxParam_") + name);
        }
        row(r)->label()->setToolTip(britishSpelling(param.value(QStringLiteral("doc")).toString(name)));
    }
    m_sections[section].enable = [this, group, effect, title] {
        const int index = m_clips.isEmpty() ? -1 : effectIndexOf(m_clips.first(), effect);
        const bool on = index >= 0 && m_clips.first().clip.value(QStringLiteral("effects")).toArray().at(index).toObject().value(QStringLiteral("enabled")).toBool(true);
        group->setEnable(on ? InspectorSection::Enable::On : InspectorSection::Enable::Off,
                         index < 0 ? QStringLiteral("Add %1 to this clip").arg(title.toLower()) : QStringLiteral("%1 on or off").arg(title));
    };
    connect(group, &InspectorSection::enableToggled, this, [this, effect](bool on) {
        if (m_clips.isEmpty()) return;
        const Clip &clip = m_clips.first();
        const int index = effectIndexOf(clip, effect);
        if (index < 0) {
            if (on) emitOps(effectParamOps(clip, effect, {}));
            return;
        }
        emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-set")}, {QStringLiteral("clip"), clip.id},
                                       {QStringLiteral("index"), index}, {QStringLiteral("enabled"), on}}});
    });
}

void InspectorPanel::buildChannelEq(QVBoxLayout *into) {
    // Low cut and presence (audio.eq, E0-MODEL), added on first edit.
    buildEffectGroup(into, QStringLiteral("channelEq"), QStringLiteral("Channel EQ"), fx(kChannelEq));
    if (InspectorSection *eq = section(QStringLiteral("channelEq"))) {
        // Open, as Volume is: the two rows the design shows.
        if (!QSettings().value(foldsKey(m_kindForSections)).toMap().contains(QStringLiteral("channelEq"))) eq->setOpen(true);
    }
}

void InspectorPanel::buildVideo(QVBoxLayout *into, bool) {
    // As the design orders them: the effect stack first (the proposal hint
    // in it), Compositing, Transform, Crop, then the folded effect groups,
    // Masks and Time.
    buildEffectsSection(into, true);
    const int compositing = addSection(into, QStringLiteral("compositing"), QStringLiteral("Compositing"), true);
    m_sections.at(compositing).section->setEnable(InspectorSection::Enable::Always, QStringLiteral("Compositing is always applied"));
    buildCompositingRows(sectionBody(compositing), compositing);
    const int transform = addSection(into, QStringLiteral("transform"), QStringLiteral("Transform"), true);
    m_sections.at(transform).section->setEnable(InspectorSection::Enable::Always, QStringLiteral("Transform is always applied"));
    buildTransformRows(sectionBody(transform), transform);
    buildFitFill(transform);
    buildCropSection(into);
    buildEffectGroup(into, QStringLiteral("distort"), QStringLiteral("Distort"), fx(kCornerPin));
    buildEffectGroup(into, QStringLiteral("stabilization"), QStringLiteral("Stabilization"), fx(kStabilizer));
    buildMasksSection(into);
    buildTimeSection(into);
}

void InspectorPanel::buildAudio(QVBoxLayout *into) {
    // A sound clip shows its sound in the Video pane too.
    buildAudioPane(into);
}

void InspectorPanel::buildAudioPane(QVBoxLayout *into) {
    if (m_clips.isEmpty()) return;
    const Clip &first = m_clips.first();
    const QJsonObject gain = defNamed(first, QStringLiteral("gain"));
    const QJsonObject pan = defNamed(first, QStringLiteral("pan"));
    // Volume: Level (gain, dB) and Pan (bipolar), then anything else the
    // catalog gives a sound clip.
    const int volume = addSection(into, QStringLiteral("volume"), QStringLiteral("Volume"), true);
    m_sections.at(volume).section->setEnable(InspectorSection::Enable::Always, QStringLiteral("Volume is always applied"));
    QVBoxLayout *body = sectionBody(volume);
    if (!gain.isEmpty()) {
        const int r = addRow(body, QStringLiteral("Level"), volume);
        bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("gain")}, gain, true)->setObjectName(QStringLiteral("prop_gain"));
    }
    if (!pan.isEmpty()) {
        const int r = addRow(body, QStringLiteral("Pan"), volume);
        bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("pan")}, pan, true)->setObjectName(QStringLiteral("prop_pan"));
    }
    for (const QJsonValue &value : defsOf(first)) {
        const QJsonObject def = value.toObject();
        const QString name = def.value(QStringLiteral("name")).toString();
        if (name == QStringLiteral("gain") || name == QStringLiteral("pan")) continue;
        const int r = addRow(body, presentationLabel(def), volume);
        if (def.value(QStringLiteral("type")).toString() == QStringLiteral("number"))
            bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, name}, def, def.contains(QStringLiteral("max")))->setObjectName(QStringLiteral("prop_") + name);
        else
            bindValue(r, row(r)->controls(), Target{Target::Kind::Prop, name}, def)->setObjectName(QStringLiteral("prop_") + name);
    }
    buildChannelEq(into);
    buildEffectsSection(into, false);
    buildTimeSection(into);
}

void InspectorPanel::addInfoHeader(QVBoxLayout *into, const QString &title) {
    auto *header = styledLabel(title, tokens::controlStrong, tokens::ink);
    header->setObjectName(QStringLiteral("infoHeader"));
    header->setFixedHeight(28);
    header->setContentsMargins(12, 0, 12, 0);
    into->addSpacing(4);
    into->addWidget(header);
}

void InspectorPanel::addInfoRow(QVBoxLayout *into, const QString &name, const QString &value, bool ui) {
    if (value.trimmed().isEmpty()) return;
    m_info.insert(name, value);
    auto *line = new QWidget;
    line->setObjectName(QStringLiteral("infoRow"));
    line->setFixedHeight(24);
    auto *layout = new QHBoxLayout(line);
    layout->setContentsMargins(12, 0, 12, 0);
    layout->setSpacing(8);
    auto *keyLabel = styledLabel(name, tokens::control, tokens::ink2);
    keyLabel->setObjectName(QStringLiteral("infoKey"));
    layout->addWidget(keyLabel);
    auto *valueLabel = styledLabel(value, ui ? tokens::control : tokens::timecode, tokens::ink);
    valueLabel->setObjectName(QStringLiteral("infoValue"));
    valueLabel->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
    valueLabel->setMinimumWidth(0);
    valueLabel->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    valueLabel->setToolTip(value);
    valueLabel->setProperty("full", value);
    valueLabel->installEventFilter(this);
    layout->addWidget(valueLabel, 1);
    into->addWidget(line);
}

void InspectorPanel::buildInfo(QVBoxLayout *into) {
    const double fps = fpsOf();
    const bool drop = m_project.value(QStringLiteral("dropFrame")).toBool();
    const auto tc = [fps, drop](qint64 frames) { return frameTimecode(frames, fps, drop); };
    if (m_mode == Mode::Sequence) {
        const QSize size = resolvedCanvasSize(m_project, m_project.value(QStringLiteral("sources")).toArray());
        double duration = m_project.value(QStringLiteral("duration")).toDouble();
        addInfoRow(into, QStringLiteral("Start"), tc(std::llround(m_project.value(QStringLiteral("start")).toDouble() * fps)));
        addInfoRow(into, QStringLiteral("Duration"), tc(std::llround(duration * fps)));
        addInfoRow(into, QStringLiteral("Frame size"), QStringLiteral("%1×%2").arg(size.width()).arg(size.height()));
        addInfoRow(into, QStringLiteral("Frame rate"), rateText(fps) + (drop ? QStringLiteral(" DF") : QString()));
        addInfoRow(into, QStringLiteral("Audio rate"), kiloHertz(m_project.value(QStringLiteral("sampleRate")).toDouble()));
        addInfoRow(into, QStringLiteral("Tracks"), QString::number(m_project.value(QStringLiteral("tracks")).toArray().size()));
        return;
    }
    if (m_clips.isEmpty()) return;
    if (m_mode != Mode::Clip) {
        addInfoRow(into, QStringLiteral("Clips"), QString::number(m_clips.size()));
        return;
    }
    const Clip &clip = m_clips.first();
    const QJsonObject source = clip.source;
    const QJsonObject info = m_mediaInfo.value(clip.clip.value(QStringLiteral("sourceId")).toString()).toObject();
    const qint64 start = qint64(clip.clip.value(QStringLiteral("startFrame")).toDouble(std::round(clip.clip.value(QStringLiteral("timelineStart")).toDouble() * fps)));
    const qint64 length = qint64(clip.clip.value(QStringLiteral("durationFrames")).toDouble(std::round(clip.clip.value(QStringLiteral("duration")).toDouble() * fps)));
    addInfoRow(into, QStringLiteral("Start"), tc(start));
    addInfoRow(into, QStringLiteral("End"), tc(start + length));
    addInfoRow(into, QStringLiteral("Duration"), tc(length));
    if (source.value(QStringLiteral("width")).toInt() > 0)
        addInfoRow(into, QStringLiteral("Frame size"), QStringLiteral("%1×%2").arg(source.value(QStringLiteral("width")).toInt()).arg(source.value(QStringLiteral("height")).toInt()));
    if (!source.isEmpty() && source.value(QStringLiteral("width")).toInt() > 0)
        addInfoRow(into, QStringLiteral("Frame rate"), rateText(source.value(QStringLiteral("fps")).toDouble()));
    // What the file says of itself (media_info); left out when it says nothing.
    addInfoRow(into, QStringLiteral("Codec"), codecText(info), true);
    QStringList picture;
    if (const QString pixFmt = info.value(QStringLiteral("pixFmt")).toString(); !pixFmt.isEmpty()) picture << pixFmt;
    if (const int bits = info.value(QStringLiteral("bitDepth")).toInt(); bits > 0) picture << QStringLiteral("%1-bit").arg(bits);
    addInfoRow(into, QStringLiteral("Pixel format"), picture.join(QStringLiteral(" · ")));
    addInfoRow(into, QStringLiteral("Colour"), colourText(info), true);
    addInfoRow(into, QStringLiteral("Camera"), info.value(QStringLiteral("camera")).toString(), true);
    addInfoRow(into, QStringLiteral("Lens"), info.value(QStringLiteral("lens")).toString(), true);
    addInfoRow(into, QStringLiteral("Reel"), info.value(QStringLiteral("reel")).toString());
    addInfoRow(into, QStringLiteral("Timecode"), info.value(QStringLiteral("timecode")).toString());
    if (source.value(QStringLiteral("hasAudio")).toBool() || source.value(QStringLiteral("channels")).toInt() > 0) {
        QStringList sound;
        if (const QString rate = kiloHertz(source.value(QStringLiteral("sampleRate")).toDouble()); !rate.isEmpty()) sound << rate;
        if (const int bits = info.value(QStringLiteral("audioBitDepth")).toInt(); bits > 0) sound << QStringLiteral("%1-bit").arg(bits);
        if (const QString channels = channelsText(source.value(QStringLiteral("channels")).toInt()); !channels.isEmpty()) sound << channels;
        addInfoRow(into, QStringLiteral("Audio"), sound.join(QStringLiteral(" · ")), true);
    }
    // Roles: the track's (the timeline gives each its role).
    QString role = clip.track.value(QStringLiteral("role")).toString();
    if (!role.isEmpty()) role[0] = role.at(0).toUpper();
    addInfoRow(into, QStringLiteral("Roles"), role, true);
    if (!source.isEmpty()) addInfoRow(into, QStringLiteral("File"), sourceFileName(source), true);
}

void InspectorPanel::buildTitle(QVBoxLayout *into) {
    const Clip &first = m_clips.first();
    const QJsonObject text = defNamed(first, QStringLiteral("text"));
    const QJsonObject font = defNamed(first, QStringLiteral("font"));
    const QJsonObject size = defNamed(first, QStringLiteral("size"));
    const QJsonObject color = defNamed(first, QStringLiteral("color"));
    const QJsonObject align = defNamed(first, QStringLiteral("align"));
    const QJsonObject y = defNamed(first, QStringLiteral("y"));
    const int words = addSection(into, QStringLiteral("text"), QStringLiteral("Text"), true);
    QVBoxLayout *body = sectionBody(words);
    if (!text.isEmpty()) {
        const int r = addRow(body, QStringLiteral("Text"), words);
        InspectorRow *textRow = row(r);
        textRow->setMinimumHeight(InspectorRow::kHeight);
        textRow->setMaximumHeight(QWIDGETSIZE_MAX);
        textRow->label()->setAlignment(Qt::AlignLeft | Qt::AlignTop);
        textRow->label()->setContentsMargins(0, 4, 0, 0);
        auto *edit = new QPlainTextEdit;
        edit->setObjectName(QStringLiteral("titleText"));
        edit->setProperty("inspector", true);
        edit->setTabChangesFocus(true);
        edit->setFont(tokens::font(tokens::control));
        edit->setStyleSheet(QStringLiteral("QPlainTextEdit { background: %1; color: %2; border: 1px solid transparent; border-radius: 4px; }"
                                           "QPlainTextEdit:focus { border-color: %3; }")
                                .arg(css(tokens::raised), css(tokens::ink), css(tokens::accent)));
        edit->setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
        const int line = QFontMetrics(edit->font()).lineSpacing();
        edit->setFixedHeight(line * 3 + 12);
        textRow->controls()->addWidget(edit, 1);
        textRow->layout()->setContentsMargins(InspectorRow::kPadLeft, 4, InspectorRow::kPadRight, 4);
        edit->installEventFilter(this);
        edit->setProperty("commit", true);
        connect(edit->document(), &QTextDocument::contentsChanged, edit, [edit, line] {
            const int lines = std::clamp(int(edit->document()->size().height()), 3, 6);
            edit->setFixedHeight(line * lines + 12);
        });
        const Target target{Target::Kind::Prop, QStringLiteral("text")};
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
    if (!font.isEmpty()) {
        const int r = addRow(body, QStringLiteral("Font"), words);
        auto *combo = new FontPopup;
        combo->setObjectName(QStringLiteral("titleFont"));
        for (const QString &family : {QStringLiteral("sans-serif"), QStringLiteral("serif"), QStringLiteral("monospace")}) {
            combo->addItem(resolvedFamily(family), family);
            combo->setItemData(combo->count() - 1, family, Qt::ToolTipRole);
        }
        combo->addItem(QStringLiteral("Other…"), QString());
        row(r)->controls()->addStretch(1);
        row(r)->controls()->addWidget(combo);
        row(r)->setWide(true);
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
            combo->updateGeometry();
        };
        binding.changed = [this, target, font] { return !sameJson(valueOf(m_clips.first(), target, font), font.value(QStringLiteral("default"))); };
        binding.resetOps = [this, target, font] { return resetOpsFor(target, font); };
        m_rows[r].values.append(binding);
    }
    if (!size.isEmpty()) {
        const int r = addRow(body, QStringLiteral("Size"), words);
        bindNumber(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("size")}, size)->setObjectName(QStringLiteral("titleSize"));
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
        field->setFixedWidth(76);
        row(r)->controls()->addStretch(1);
        row(r)->controls()->addWidget(field);
        row(r)->setWide(true);
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
    buildEffectsSection(into, false);
    const int compositing = addSection(into, QStringLiteral("compositing"), QStringLiteral("Compositing"));
    m_sections.at(compositing).section->setEnable(InspectorSection::Enable::Always, QStringLiteral("Compositing is always applied"));
    buildCompositingRows(sectionBody(compositing), compositing);
    const int transform = addSection(into, QStringLiteral("transform"), QStringLiteral("Transform"));
    m_sections.at(transform).section->setEnable(InspectorSection::Enable::Always, QStringLiteral("Transform is always applied"));
    buildTransformRows(sectionBody(transform), transform);
    buildCropSection(into);
    if (m_maskable) buildMasksSection(into);
    buildTimeSection(into);
}

void InspectorPanel::buildSolid(QVBoxLayout *into) {
    const QJsonObject color = defNamed(m_clips.first(), QStringLiteral("color"));
    buildEffectsSection(into, false);
    const int compositing = addSection(into, QStringLiteral("compositing"), QStringLiteral("Compositing"), true);
    m_sections.at(compositing).section->setEnable(InspectorSection::Enable::Always, QStringLiteral("Compositing is always applied"));
    if (!color.isEmpty()) {
        const int r = addRow(sectionBody(compositing), QStringLiteral("Colour"), compositing);
        bindValue(r, row(r)->controls(), Target{Target::Kind::Prop, QStringLiteral("color")}, color)->setObjectName(QStringLiteral("solidColor"));
    }
    buildCompositingRows(sectionBody(compositing), compositing);
    const int transform = addSection(into, QStringLiteral("transform"), QStringLiteral("Transform"));
    m_sections.at(transform).section->setEnable(InspectorSection::Enable::Always, QStringLiteral("Transform is always applied"));
    buildTransformRows(sectionBody(transform), transform);
    buildCropSection(into);
    buildMasksSection(into);
    buildTimeSection(into);
}

void InspectorPanel::buildAdjustment(QVBoxLayout *into) {
    buildEffectsSection(into, true);
    const int compositing = addSection(into, QStringLiteral("compositing"), QStringLiteral("Compositing"), true);
    m_sections.at(compositing).section->setEnable(InspectorSection::Enable::Always, QStringLiteral("Compositing is always applied"));
    buildCompositingRows(sectionBody(compositing), compositing);
    const int transform = addSection(into, QStringLiteral("transform"), QStringLiteral("Transform"));
    m_sections.at(transform).section->setEnable(InspectorSection::Enable::Always, QStringLiteral("Transform is always applied"));
    buildTransformRows(sectionBody(transform), transform);
    buildCropSection(into);
    buildMasksSection(into);
    buildTimeSection(into);
}

void InspectorPanel::buildCaption(QVBoxLayout *into) {
    // Captions carry no properties; the text is read-only until the editor
    // can set it.
    const int words = addSection(into, QStringLiteral("caption"), QStringLiteral("Caption"), true);
    auto *text = styledLabel(QString(), tokens::control, tokens::ink);
    text->setObjectName(QStringLiteral("captionText"));
    text->setWordWrap(true);
    text->setContentsMargins(12, 4, 12, 4);
    text->setTextInteractionFlags(Qt::TextSelectableByMouse);
    sectionBody(words)->addWidget(text);
    auto *hint = new LinkButton(QStringLiteral("Ask Kerf to change it"), QStringLiteral("g-ask"));
    hint->setToolTip(QStringLiteral("Ask Kerf to change the caption's text"));
    hint->setObjectName(QStringLiteral("captionHint"));
    connect(hint, &QPushButton::clicked, this, [this] { askAbout(QLatin1Char('@') + displayName(m_clips.first()) + QStringLiteral(" caption text:")); });
    auto *line = new QHBoxLayout;
    line->setContentsMargins(4, 0, 12, 4);
    line->addWidget(hint);
    line->addStretch(1);
    sectionBody(words)->addLayout(line);
    m_refreshers.append([this, text] { text->setText(m_clips.first().clip.value(QStringLiteral("text")).toString()); });
    buildTimeSection(into);
}

void InspectorPanel::buildGenerator(QVBoxLayout *into) {
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
    buildEffectsSection(into, false);
    const int compositing = addSection(into, QStringLiteral("compositing"), QStringLiteral("Compositing"));
    m_sections.at(compositing).section->setEnable(InspectorSection::Enable::Always, QStringLiteral("Compositing is always applied"));
    buildCompositingRows(sectionBody(compositing), compositing);
    const int transform = addSection(into, QStringLiteral("transform"), QStringLiteral("Transform"));
    m_sections.at(transform).section->setEnable(InspectorSection::Enable::Always, QStringLiteral("Transform is always applied"));
    buildTransformRows(sectionBody(transform), transform);
    buildCropSection(into);
    if (m_maskable) buildMasksSection(into);
    buildTimeSection(into);
}

void InspectorPanel::buildSequence(QVBoxLayout *into) {
    const EditorTheme &theme = editorTheme();
    const double fps = fpsOf();
    const bool drop = m_project.value(QStringLiteral("dropFrame")).toBool();
    const auto plain = [](const QString &value, const QString &name) {
        auto *label = styledLabel(value, tokens::control, tokens::ink);
        label->setObjectName(name);
        label->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
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
        row(r)->controls()->addStretch(1);
        row(r)->controls()->addWidget(size);
        row(r)->setWide(true);
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
        line->setFixedHeight(InspectorRow::kHeight);
        auto *layout = new QHBoxLayout(line);
        layout->setContentsMargins(InspectorRow::kPadLeft, 0, InspectorRow::kPadRight, 0);
        layout->setSpacing(8);
        auto *square = new KindSquare;
        square->setColor(theme.clip(trackKind(track)).header);
        layout->addWidget(square);
        const TrackBadge badge = trackBadge(m_project, track.value(QStringLiteral("id")).toString());
        auto *id = styledLabel(badge.badge, tokens::timecode, tokens::ink);
        id->setFixedWidth(28);
        layout->addWidget(id);
        auto *name = styledLabel(badge.name, tokens::control, tokens::ink2);
        name->setObjectName(QStringLiteral("sequenceTrackName"));
        name->setMinimumWidth(0);
        name->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
        layout->addWidget(name, 1);
        const int count = int(track.value(QStringLiteral("clips")).toArray().size());
        layout->addWidget(styledLabel(count == 1 ? QStringLiteral("1 clip") : QStringLiteral("%1 clips").arg(count), tokens::meta, tokens::ink3));
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
        line->setFixedHeight(InspectorRow::kHeight);
        auto *layout = new QHBoxLayout(line);
        layout->setContentsMargins(InspectorRow::kPadLeft, 0, InspectorRow::kPadRight, 0);
        layout->setSpacing(8);
        auto *dot = new KindSquare;
        dot->setColor(theme.marker(marker.value(QStringLiteral("color")).toString()));
        layout->addWidget(dot);
        auto *time = styledLabel(frameTimecode(std::llround(marker.value(QStringLiteral("time")).toDouble() * fps), fps, drop), tokens::timecode, tokens::ink);
        time->setObjectName(QStringLiteral("markerTime"));
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
    if (markers.isEmpty()) {
        QLabel *none = quietLabel(QStringLiteral("No markers"));
        none->setContentsMargins(12, 4, 12, 4);
        sectionBody(markerSection)->addWidget(none);
    }
}

// ---------------------------------------------------------------- wheels

void InspectorPanel::buildWheels(QVBoxLayout *into) {
    // The Finish workspace's Lift / Gamma / Gain (spec 5.1): pad 12 16 8,
    // spread across, a line under them. They write the clip's ASC CDL
    // (offset, power, slope), or its Lumetri wheels when Lumetri is its
    // colour effect and no CDL is.
    auto *pane = new LinePane(false, true);
    pane->setObjectName(QStringLiteral("inspectorWheels"));
    auto *layout = new QHBoxLayout(pane);
    layout->setContentsMargins(16, 12, 16, 8);
    layout->setSpacing(0);
    m_wheels = pane;
    const QJsonObject cdlDef = m_effectDefs.value(fx(kCdl));
    const auto paramDef = [](const QJsonObject &effect, const QString &name) {
        for (const QJsonValue &param : effect.value(QStringLiteral("params")).toArray())
            if (param.toObject().value(QStringLiteral("name")).toString() == name) return param.toObject();
        return QJsonObject{};
    };
    // Which effect the wheels write: CDL unless the stack grades in Lumetri only.
    const auto lumetri = [this] {
        return !m_clips.isEmpty() && effectIndexOf(m_clips.first(), fx(kCdl)) < 0 && effectIndexOf(m_clips.first(), fx(kLumetri)) >= 0;
    };
    for (int index = 0; index < 3; ++index) {
        const WheelSpec spec = kWheels[index];
        if (index > 0) layout->addStretch(1);
        auto *column = new QWidget;
        column->setObjectName(QStringLiteral("wheel_%1").arg(QString::fromLatin1(spec.label).toLower()));
        column->setFixedWidth(ColorWheel::kSize);
        auto *stack = new QVBoxLayout(column);
        stack->setContentsMargins(0, 0, 0, 0);
        stack->setSpacing(0);
        auto *wheel = new ColorWheel;
        wheel->setObjectName(QStringLiteral("colorWheel_%1").arg(QString::fromLatin1(spec.label).toLower()));
        ui::setTip(wheel, QStringLiteral("%1: drag to tint, double-click to centre").arg(QString::fromLatin1(spec.label)));
        stack->addWidget(wheel);
        stack->addSpacing(4);
        auto *label = styledLabel(QString::fromLatin1(spec.label), tokens::meta, tokens::ink2);
        label->setAlignment(Qt::AlignHCenter);
        label->setFixedHeight(16);
        stack->addWidget(label);
        auto *value = newScrubField();
        value->setObjectName(QStringLiteral("wheelValue_%1").arg(QString::fromLatin1(spec.label).toLower()));
        value->setSmall(true);
        value->setFixedHeight(16);
        value->setFixedWidth(ColorWheel::kSize);
        stack->addWidget(value);
        layout->addWidget(column);
        const QString cdl = QString::fromLatin1(spec.cdl);
        const QString lum = QString::fromLatin1(spec.lumetri);
        // The wheel's state now: its master value and its puck.
        const auto read = [this, spec, cdl, lum, cdlDef, paramDef, lumetri](double *master, QPointF *puck) {
            const Clip &clip = m_clips.first();
            if (lumetri()) {
                const QJsonObject def = m_effectDefs.value(fx(kLumetri));
                const auto at = [&](const QString &suffix) {
                    const QString name = QStringLiteral("wheel_%1_%2").arg(lum, suffix);
                    return valueOf(clip, Target{Target::Kind::Fx, name, -1, QString(), fx(kLumetri)}, paramDef(def, name)).toDouble();
                };
                *master = at(QStringLiteral("l"));
                *puck = QPointF(at(QStringLiteral("x")), at(QStringLiteral("y")));
                return;
            }
            const auto channel = [&](const char *c) {
                const QString name = QStringLiteral("%1_%2").arg(QString::fromLatin1(c), cdl);
                return valueOf(clip, Target{Target::Kind::Fx, name, -1, QString(), fx(kCdl)}, paramDef(cdlDef, name)).toDouble(spec.neutral);
            };
            toWheel(channel("r"), channel("g"), channel("b"), spec.neutral, spec.range, master, puck);
        };
        // The edit for a master value and a puck.
        const auto write = [this, spec, cdl, lum, cdlDef, paramDef, lumetri](double master, const QPointF &puck) {
            if (m_clips.isEmpty()) return;
            const Clip &clip = m_clips.first();
            QJsonObject params;
            if (lumetri()) {
                params.insert(QStringLiteral("wheel_%1_x").arg(lum), round4(std::clamp(puck.x(), -1.0, 1.0)));
                params.insert(QStringLiteral("wheel_%1_y").arg(lum), round4(std::clamp(puck.y(), -1.0, 1.0)));
                params.insert(QStringLiteral("wheel_%1_l").arg(lum), std::round(std::clamp(master, -100.0, 100.0) * 10.0) / 10.0);
                for (auto it = params.begin(); it != params.end(); ++it)
                    if (std::abs(it.value().toDouble()) < 1e-9) it.value() = QJsonValue();
                emitOps(effectParamOps(clip, fx(kLumetri), params));
                return;
            }
            double r = 0, g = 0, b = 0;
            fromWheel(master, puck, spec.range, &r, &g, &b);
            const struct {
                const char *channel;
                double value;
            } channels[] = {{"r", r}, {"g", g}, {"b", b}};
            for (const auto &channel : channels) {
                const QString name = QStringLiteral("%1_%2").arg(QString::fromLatin1(channel.channel), cdl);
                const QJsonObject def = paramDef(cdlDef, name);
                const double clamped = round4(std::clamp(channel.value, def.value(QStringLiteral("min")).toDouble(-10.0), def.value(QStringLiteral("max")).toDouble(10.0)));
                params.insert(name, std::abs(clamped - spec.neutral) < 1e-9 ? QJsonValue() : QJsonValue(clamped));
            }
            emitOps(effectParamOps(clip, fx(kCdl), params));
        };
        NumberFormat format;
        format.decimals = 2;
        format.step = 0.01;
        format.bigStep = 0.1;
        format.smallStep = 0.001;
        format.showPlus = spec.deviation;
        m_refreshers.append([this, wheel, value, read, spec, format, lumetri] {
            if (m_clips.isEmpty()) return;
            double master = 0;
            QPointF puck;
            read(&master, &puck);
            wheel->setPuck(puck);
            NumberFormat shown = format;
            if (lumetri()) {
                shown.decimals = 0;
                shown.step = 1.0;
                shown.bigStep = 10.0;
                shown.smallStep = 0.1;
                shown.showPlus = true;
                shown.min = -100.0;
                shown.max = 100.0;
                value->setFormat(shown);
                value->setValue(master);
            } else {
                // Lift and Gamma read as their change, Gain as its value.
                shown.min = spec.deviation ? -spec.neutral - 1.0 : 0.0;
                shown.max = 10.0 - (spec.deviation ? spec.neutral : 0.0);
                value->setFormat(shown);
                value->setValue(spec.deviation ? master - spec.neutral : master);
            }
            value->setToolTip(QStringLiteral("%1: drag or type the %2").arg(QString::fromLatin1(spec.label), lumetri() ? QStringLiteral("lightness")
                                                                                                                    : QStringLiteral("level")));
        });
        connect(wheel, &ColorWheel::puckReleased, this, [read, write](const QPointF &puck, const QPointF &) {
            double master = 0;
            QPointF was;
            read(&master, &was);
            write(master, puck);
        });
        connect(value, &ScrubField::committed, this, [read, write, spec, lumetri](double shown, double, bool) {
            double master = 0;
            QPointF puck;
            read(&master, &puck);
            write(lumetri() ? shown : spec.deviation ? shown + spec.neutral : shown, puck);
        });
    }
    into->addWidget(pane);
}

// ---------------------------------------------------------- proposal hint

void InspectorPanel::buildProposalHint() {
    // .prow0: 20 tall, pad 0 8 0 32, gap 8, UI 11 ink2: a dot, what the
    // proposal would change, and Show.
    auto *hint = new QWidget;
    hint->setObjectName(QStringLiteral("inspectorProposalHint"));
    hint->setFixedHeight(20);
    auto *layout = new QHBoxLayout(hint);
    layout->setContentsMargins(32, 0, 8, 0);
    layout->setSpacing(8);
    auto *dot = new ui::Dot(col(tokens::ink2));
    layout->addWidget(dot, 0, Qt::AlignVCenter);
    m_hintText = styledLabel(QString(), tokens::meta, tokens::ink2);
    m_hintText->setObjectName(QStringLiteral("proposalHintText"));
    m_hintText->setMinimumWidth(0);
    m_hintText->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    layout->addWidget(m_hintText, 1);
    auto *show = new LinkButton(QStringLiteral("Show"), QString(), 20, tokens::meta);
    show->setObjectName(QStringLiteral("proposalHintShow"));
    show->setToolTip(QStringLiteral("Open the proposal's thread"));
    connect(show, &QPushButton::clicked, this, [this] {
        if (!m_hintProposal.isEmpty()) emit proposalShowRequested(m_hintThread, m_hintProposal);
    });
    layout->addWidget(show);
    hint->hide();
    m_hint = hint;
    m_bodyWidget->layout()->addWidget(hint);
}

void InspectorPanel::placeProposalHint() {
    if (!m_hint) return;
    const auto hide = [this] {
        m_hint->hide();
        m_hintProposal.clear();
        m_hintThread.clear();
    };
    if (m_mode != Mode::Clip || m_pane != Pane::Video || m_clips.isEmpty()) return hide();
    const Clip &clip = m_clips.first();
    // The first open proposal whose timeline changes this clip's properties
    // or effects, and the first such change.
    for (const QJsonValue &value : m_proposals) {
        const QJsonObject proposal = value.toObject();
        if (proposal.value(QStringLiteral("status")).toString(QStringLiteral("pending")) != QStringLiteral("pending")) continue;
        const QString id = proposal.value(QStringLiteral("proposalId")).toString();
        const QJsonObject timeline = m_proposalTimelines.value(id).toObject();
        QJsonObject proposed;
        for (const QJsonValue &track : timeline.value(QStringLiteral("tracks")).toArray())
            for (const QJsonValue &other : track.toObject().value(QStringLiteral("clips")).toArray())
                if (other.toObject().value(QStringLiteral("id")).toString() == clip.id) proposed = other.toObject();
        if (proposed.isEmpty()) continue;
        QString change;
        int group = -1;
        // Its own properties.
        const QJsonObject now = clip.clip.value(QStringLiteral("props")).toObject();
        const QJsonObject then = proposed.value(QStringLiteral("props")).toObject();
        QStringList names = now.keys();
        for (const QString &name : then.keys())
            if (!names.contains(name)) names << name;
        int changes = 0;
        for (const QString &name : std::as_const(names)) {
            if (now.value(name) == then.value(name)) continue;
            ++changes;
            if (!change.isEmpty()) continue;
            const QJsonObject def = defNamed(clip, name);
            const QJsonValue next = then.value(name).isUndefined() || then.value(name).isNull() ? def.value(QStringLiteral("default")) : then.value(name);
            if (def.value(QStringLiteral("type")).toString() == QStringLiteral("number") && next.isDouble()) {
                const NumberFormat format = formatFor(clip, def);
                change = QStringLiteral("%1 %2").arg(presentationLabel(def), formatWithUnit(format, next.toDouble() * format.scale));
            } else {
                change = QStringLiteral("%1 %2").arg(presentationLabel(def), titleCase(next.toVariant().toString()));
            }
            group = m_targetSections.value(targetKey(Target{Target::Kind::Prop, name}), -1);
        }
        // Its effects.
        const QJsonArray nowFx = clip.clip.value(QStringLiteral("effects")).toArray();
        const QJsonArray thenFx = proposed.value(QStringLiteral("effects")).toArray();
        const int effectsGroup = section(QStringLiteral("effects")) ? int(std::find_if(m_sections.cbegin(), m_sections.cend(), [](const SectionEntry &e) {
                                                                              return e.section->id() == QStringLiteral("effects");
                                                                          }) - m_sections.cbegin())
                                                                    : -1;
        for (int index = 0; index < std::max(nowFx.size(), thenFx.size()); ++index) {
            const QJsonObject a = nowFx.at(index).toObject();
            const QJsonObject b = thenFx.at(index).toObject();
            if (a == b) continue;
            ++changes;
            if (!change.isEmpty()) continue;
            group = effectsGroup;
            const QString bfx = b.value(QStringLiteral("fx")).toString();
            const QString name = presentationEffectName(m_effectDefs.value(bfx.isEmpty() ? a.value(QStringLiteral("fx")).toString() : bfx)
                                                            .value(QStringLiteral("name")).toString(bfx));
            if (b.isEmpty()) {
                change = QStringLiteral("removing %1").arg(name);
            } else if (a.value(QStringLiteral("fx")) != b.value(QStringLiteral("fx"))) {
                change = name;
            } else {
                const QJsonObject pa = a.value(QStringLiteral("params")).toObject();
                const QJsonObject pb = b.value(QStringLiteral("params")).toObject();
                QStringList params = pb.keys();
                for (const QString &p : pa.keys())
                    if (!params.contains(p)) params << p;
                for (const QString &p : std::as_const(params)) {
                    if (pa.value(p) == pb.value(p)) continue;
                    QJsonObject def;
                    for (const QJsonValue &d : m_effectDefs.value(bfx).value(QStringLiteral("params")).toArray())
                        if (d.toObject().value(QStringLiteral("name")).toString() == p) def = d.toObject();
                    const QJsonValue next = pb.value(p).isUndefined() || pb.value(p).isNull() ? def.value(QStringLiteral("default")) : pb.value(p);
                    if (next.isDouble()) {
                        const NumberFormat format = formatFor(clip, def);
                        change = QStringLiteral("%1 %2").arg(withoutSharedWord(presentationLabel(def), name), formatWithUnit(format, next.toDouble() * format.scale));
                    } else {
                        change = QStringLiteral("%1 %2").arg(withoutSharedWord(presentationLabel(def), name), next.toVariant().toString());
                    }
                    break;
                }
                if (change.isEmpty())
                    change = a.value(QStringLiteral("enabled")).toBool(true) != b.value(QStringLiteral("enabled")).toBool(true)
                                 ? QStringLiteral("%1 %2").arg(name, b.value(QStringLiteral("enabled")).toBool(true) ? QStringLiteral("on") : QStringLiteral("off"))
                                 : name;
            }
        }
        if (change.isEmpty()) continue;
        if (changes > 1) change += QStringLiteral(" +%1").arg(changes - 1);
        const QString author = proposal.value(QStringLiteral("author")).toObject().value(QStringLiteral("name")).toString(QStringLiteral("Kerf"));
        m_hintText->setText(QStringLiteral("%1 proposes %2").arg(author.isEmpty() ? QStringLiteral("Kerf") : author, change));
        m_hintText->setToolTip(m_hintText->text());
        m_hintProposal = id;
        m_hintThread = m_threadOf ? m_threadOf(id) : QString();
        if (m_hintThread.isEmpty()) m_hintThread = QStringLiteral("proposal:") + id;
        // Into the group of what it changes (Effects for an effect), else
        // at the body's end.
        QVBoxLayout *target = group >= 0 && group < m_sections.size() ? sectionBody(group) : static_cast<QVBoxLayout *>(m_bodyWidget->layout());
        if (m_hint->parentWidget() != target->parentWidget()) {
            if (QLayout *old = m_hint->parentWidget() ? m_hint->parentWidget()->layout() : nullptr) old->removeWidget(m_hint);
            target->addWidget(m_hint);
        }
        m_hint->show();
        return;
    }
    hide();
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
        button->setFixedSize(tokens::height::h24, tokens::height::h24);
        masks->addTrailing(button);
    }
    const QJsonArray list = m_clips.first().clip.value(QStringLiteral("masks")).toArray();
    masks->setCount(int(list.size()));
    if (list.isEmpty()) {
        auto *hint = quietLabel(QStringLiteral("Draw a mask on the picture to show only part of this clip."));
        hint->setObjectName(QStringLiteral("masksHint"));
        hint->setWordWrap(true);
        hint->setContentsMargins(12, 4, 12, 4);
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
    row->setContentsMargins(InspectorRow::kPadLeft, 0, InspectorRow::kPadRight, 0);
    row->setSpacing(6);
    const QJsonObject mask = [this, &name] {
        for (const QJsonValue &value : m_clips.first().clip.value(QStringLiteral("masks")).toArray())
            if (value.toObject().value(QStringLiteral("name")).toString() == name) return value.toObject();
        return QJsonObject{};
    }();
    auto *shape = new QLabel;
    shape->setFixedSize(16, 16);
    const QString icon = mask.contains(QStringLiteral("ellipse")) ? QStringLiteral("g-mell")
                         : mask.contains(QStringLiteral("points")) ? QStringLiteral("g-mpen")
                                                                   : QStringLiteral("g-mrect");
    shape->setPixmap(icons::pixmap(icon, col(tokens::ink2), 16, devicePixelRatioF()));
    row->addWidget(shape);
    auto *select = new QToolButton;
    select->setObjectName(QStringLiteral("maskName"));
    select->setText(name);
    select->setCheckable(true);
    select->setAutoRaise(true);
    select->setToolTip(QStringLiteral("Show this mask's handles on the picture"));
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
    row->addWidget(styledLabel(QStringLiteral("Invert"), tokens::control, tokens::ink2));
    auto *more = new GlyphButton(GlyphButton::Glyph::More);
    more->setObjectName(QStringLiteral("maskMore"));
    auto *menu = new QMenu(box);
    ui::styleMenu(menu);
    menu->addAction(QStringLiteral("Ask Kerf about this mask"), this, [this, name] {
        askAbout(QStringLiteral("@%1 mask %2").arg(displayName(m_clips.first()), name));
    })->setObjectName(QStringLiteral("maskAsk"));
    menu->addSeparator();
    QAction *remove = menu->addAction(QStringLiteral("Remove"), this, [this, name] {
        emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("mask-remove")}, {QStringLiteral("clip"), m_clips.first().id},
                                       {QStringLiteral("name"), name}}});
    });
    remove->setObjectName(QStringLiteral("maskRemove"));
    ui::setMenuRole(remove, ui::MenuRole::Warn);
    connect(more, &QAbstractButton::clicked, this, [more, menu] { ui::popupMenu(menu, more); });
    row->addWidget(more);
    auto *chevron = new GlyphButton(GlyphButton::Glyph::Chevron);
    chevron->setObjectName(QStringLiteral("maskChevron"));
    row->addWidget(chevron);
    layout->addWidget(header);
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
        this->row(r)->controls()->addStretch(1);
        this->row(r)->controls()->addWidget(mode);
        this->row(r)->setWide(true);
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
                mode->updateGeometry();
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
        if (def.value(QStringLiteral("type")).toString() == QStringLiteral("number"))
            bindNumber(r, this->row(r)->controls(), target, def)->setObjectName(QStringLiteral("maskParam_") + param);
        else
            bindValue(r, this->row(r)->controls(), target, def)->setObjectName(QStringLiteral("maskParam_") + param);
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
    // A sound clip is offered the sound effects; one that carries none and
    // is offered none gets no Effects group at all.
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
        ui::setTip(add, QStringLiteral("Add an effect"));
        connect(add, &QAbstractButton::clicked, this, [this, sound] { emit addEffectRequested(sound); });
        effects->addTrailing(add);
    }
    if (m_clips.size() > 1) {
        m_sections[section].summary = [this] {
            int carrying = 0;
            for (const Clip &clip : std::as_const(m_clips)) carrying += clip.clip.value(QStringLiteral("effects")).toArray().isEmpty() ? 0 : 1;
            return carrying == 0 ? QString() : QStringLiteral("On %1 of %2 clips").arg(carrying).arg(m_clips.size());
        };
        for (const Clip &clip : std::as_const(m_clips)) {
            QStringList names;
            for (const QJsonValue &effect : clip.clip.value(QStringLiteral("effects")).toArray()) {
                const QString id = effect.toObject().value(QStringLiteral("fx")).toString();
                names << m_effectDefs.value(id).value(QStringLiteral("name")).toString(id);
            }
            if (names.isEmpty()) continue;
            auto *line = quietLabel(QStringLiteral("%1: %2").arg(displayName(clip), names.join(QStringLiteral(", "))));
            line->setWordWrap(true);
            line->setContentsMargins(12, 4, 12, 4);
            sectionBody(section)->addWidget(line);
        }
        if (offered) {
            auto *addAll = new LinkButton(QStringLiteral("Add effect to all"), QStringLiteral("g-fx"));
            addAll->setObjectName(QStringLiteral("addEffect"));
            connect(addAll, &QPushButton::clicked, this, [this, sound] { emit addEffectRequested(sound); });
            auto *line = new QHBoxLayout;
            line->setContentsMargins(4, 0, 8, 0);
            line->addWidget(addAll);
            line->addStretch(1);
            sectionBody(section)->addLayout(line);
        }
        return;
    }
    const QJsonArray list = m_clips.first().clip.value(QStringLiteral("effects")).toArray();
    effects->setCount(int(list.size()));
    for (int index = 0; index < list.size(); ++index) buildEffect(sectionBody(section), index);
    if (offered) {
        auto *addEffect = new LinkButton(QStringLiteral("Add effect"), QStringLiteral("g-fx"));
        addEffect->setObjectName(QStringLiteral("addEffect"));
        connect(addEffect, &QPushButton::clicked, this, [this, sound] { emit addEffectRequested(sound); });
        auto *line = new QHBoxLayout;
        line->setContentsMargins(4, 0, 8, 0);
        line->addWidget(addEffect);
        line->addStretch(1);
        sectionBody(section)->addLayout(line);
    }
    // The group's box: every effect on, or off (each effect keeps its own).
    m_sections[section].enable = [this, effects] {
        const QJsonArray stack = m_clips.isEmpty() ? QJsonArray{} : m_clips.first().clip.value(QStringLiteral("effects")).toArray();
        if (stack.isEmpty()) {
            effects->setEnable(InspectorSection::Enable::Hidden);
            return;
        }
        const bool any = std::any_of(stack.cbegin(), stack.cend(), [](const QJsonValue &e) { return e.toObject().value(QStringLiteral("enabled")).toBool(true); });
        effects->setEnable(any ? InspectorSection::Enable::On : InspectorSection::Enable::Off, QStringLiteral("Every effect on or off"));
    };
    connect(effects, &InspectorSection::enableToggled, this, [this](bool on) {
        if (m_clips.isEmpty()) return;
        QJsonArray ops;
        const QJsonArray stack = m_clips.first().clip.value(QStringLiteral("effects")).toArray();
        for (int index = 0; index < stack.size(); ++index)
            if (stack.at(index).toObject().value(QStringLiteral("enabled")).toBool(true) != on)
                ops.append(QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-set")}, {QStringLiteral("clip"), m_clips.first().id},
                                       {QStringLiteral("index"), index}, {QStringLiteral("enabled"), on}});
        emitOps(ops);
    });
    // The tooltip: the effects that are on, the ones off counted.
    m_sections[section].items = [this](InspectorSection *into) {
        QStringList on;
        QStringList all;
        int off = 0;
        for (const QJsonValue &effect : m_clips.first().clip.value(QStringLiteral("effects")).toArray()) {
            const QJsonObject e = effect.toObject();
            const QString id = e.value(QStringLiteral("fx")).toString();
            QString name = m_effectDefs.value(id).value(QStringLiteral("name")).toString();
            if (name.isEmpty()) name = e.value(QStringLiteral("doc")).toString(id).section(QStringLiteral(": "), 0, 0).remove(QLatin1Char('.'));
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
    const QString id = effect.value(QStringLiteral("fx")).toString();
    const QJsonObject def = m_effectDefs.value(id);
    const QString doc = effect.value(QStringLiteral("doc")).toString();
    QString name = def.value(QStringLiteral("name")).toString();
    if (name.isEmpty() && !doc.isEmpty()) name = doc.section(QStringLiteral(": "), 0, 0).remove(QLatin1Char('.'));
    if (name.isEmpty()) name = id;
    else name = presentationEffectName(name);
    auto *box = new QWidget;
    box->setObjectName(QStringLiteral("effectBox"));
    box->setProperty("fx", id);
    box->setProperty("index", index);
    auto *layout = new QVBoxLayout(box);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    // The effect's own row (.l.w): its enable box and name in ink across the
    // label and control columns, its More and fold at the right end.
    auto *header = new QWidget;
    header->setObjectName(QStringLiteral("effectHeader"));
    header->setFixedHeight(InspectorRow::kHeight);
    header->setFocusPolicy(Qt::StrongFocus);
    header->setCursor(Qt::PointingHandCursor);
    header->setProperty("index", index);
    header->installEventFilter(this);
    auto *row = new QHBoxLayout(header);
    row->setContentsMargins(InspectorRow::kPadLeft, 0, InspectorRow::kPadRight, 0);
    row->setSpacing(6);
    // The effect's enable box: the kit check box, as every switch is.
    auto *enabled = new InspectorSwitch;
    enabled->setObjectName(QStringLiteral("effectEnabled"));
    enabled->setAccessibleName(QStringLiteral("On"));
    ui::setTip(enabled, QStringLiteral("%1 on or off").arg(name));
    connect(enabled, &QAbstractButton::clicked, this, [this, index](bool on) {
        emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-set")}, {QStringLiteral("clip"), m_clips.first().id},
                                       {QStringLiteral("index"), index}, {QStringLiteral("enabled"), on}}});
    });
    row->addWidget(enabled);
    auto *title = styledLabel(name, tokens::control, tokens::ink);
    title->setObjectName(QStringLiteral("effectName"));
    title->setMinimumWidth(0);
    title->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    title->setToolTip(britishSpelling(def.value(QStringLiteral("doc")).toString(doc)));
    row->addWidget(title, 1);
    auto *more = new GlyphButton(GlyphButton::Glyph::More);
    more->setObjectName(QStringLiteral("effectMore"));
    ui::setTip(more, QStringLiteral("Reset, ask Kerf, remove"));
    row->addWidget(more);
    auto *chevron = new GlyphButton(GlyphButton::Glyph::Chevron);
    chevron->setObjectName(QStringLiteral("effectChevron"));
    row->addWidget(chevron);
    layout->addWidget(header);
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
    const QJsonArray defs = def.value(QStringLiteral("params")).toArray();
    QStringList groups;
    for (const QJsonValue &value : defs) {
        const QString group = presentationGroup(value.toObject());
        if (!group.isEmpty() && !groups.contains(group)) groups << group;
    }
    // Labels a first word the effect's name already says loses ("Threshold"
    // under Edge Glow), unless two rows would then read alike.
    QStringList labels;
    for (const QJsonValue &value : defs) labels << presentationLabel(value.toObject());
    QStringList shortLabels;
    for (const QString &label : std::as_const(labels)) shortLabels << withoutSharedWord(label, name);
    for (qsizetype at = 0; at < shortLabels.size(); ++at)
        if (shortLabels.count(shortLabels.at(at)) > 1 || (labels.contains(shortLabels.at(at)) && shortLabels.at(at) != labels.at(at)))
            shortLabels[at] = labels.at(at);
    const int rowsShown = defs.size() - kEffectRowsShown >= 2 ? kEffectRowsShown : int(defs.size());
    int shownRows = 0;
    int hidden = 0;
    QString lastGroup;
    int paramIndex = -1;
    for (const QJsonValue &value : defs) {
        const QJsonObject param = value.toObject();
        ++paramIndex;
        const QString group = presentationGroup(param);
        const bool inMore = groups.isEmpty() ? shownRows >= rowsShown : group != groups.first();
        QVBoxLayout *target = inMore ? moreLayout : paramsLayout;
        if (!group.isEmpty() && group != lastGroup) {
            auto *sub = styledLabel(group, tokens::meta, tokens::ink3);
            sub->setObjectName(QStringLiteral("effectGroup"));
            sub->setContentsMargins(InspectorRow::kPadLeft, 6, 12, 2);
            target->addWidget(sub);
            lastGroup = group;
            if (inMore) ++hidden;
        }
        if (inMore && groups.isEmpty()) ++hidden;
        else if (!inMore) ++shownRows;
        const QString paramName = param.value(QStringLiteral("name")).toString();
        const int r = addRow(target, shortLabels.at(paramIndex));
        // Effect rows count towards the Effects group (its reset, its hint).
        m_rowSections.insert(r, int(m_sections.size()) - 1);
        const Target t{Target::Kind::Effect, paramName, index};
        if (param.value(QStringLiteral("type")).toString() == QStringLiteral("number")) {
            const bool bounded = param.contains(QStringLiteral("min")) && param.contains(QStringLiteral("max"));
            bindNumber(r, this->row(r)->controls(), t, param, bounded)->setObjectName(QStringLiteral("effectParam_") + paramName);
        } else {
            bindValue(r, this->row(r)->controls(), t, param)->setObjectName(QStringLiteral("effectParam_") + paramName);
        }
        this->row(r)->label()->setToolTip(britishSpelling(param.value(QStringLiteral("doc")).toString(paramName)));
    }
    paramsLayout->addWidget(moreRows);
    const QString moreKey = QStringLiteral("editor/inspector/fx/%1/more").arg(id);
    if (hidden > 0) {
        auto *toggle = new LinkButton(QString(), QString(), 20, tokens::meta);
        toggle->setObjectName(QStringLiteral("effectMoreToggle"));
        const auto label = [toggle, hidden](bool open) { toggle->setText(open ? QStringLiteral("Fewer") : QStringLiteral("%1 more").arg(hidden)); };
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
        toggleRow->setContentsMargins(InspectorRow::kPadLeft - 8 + 84 + 8, 2, 0, 0);
        toggleRow->addWidget(toggle);
        toggleRow->addStretch(1);
        paramsLayout->addLayout(toggleRow);
    } else {
        moreRows->hide();
    }
    if (def.isEmpty() && doc.isEmpty()) {
        auto *missing = quietLabel(QStringLiteral("Not in this version of Kerf"));
        missing->setObjectName(QStringLiteral("effectMissing"));
        missing->setContentsMargins(InspectorRow::kPadLeft + 20, 0, 12, 4);
        layout->addWidget(missing);
    }
    layout->addWidget(params);
    const QString openKey = QStringLiteral("editor/inspector/fx/%1/open").arg(id);
    const auto setOpen = [params, chevron](bool open) {
        params->setVisible(open);
        chevron->setOpen(open);
    };
    setOpen(QSettings().value(openKey, false).toBool() && !defs.isEmpty());
    const auto toggleOpen = [params, setOpen, openKey] {
        const bool now = params->isHidden();
        setOpen(now);
        QSettings().setValue(openKey, now);
    };
    connect(chevron, &QAbstractButton::clicked, this, toggleOpen);
    auto *menu = new QMenu(box);
    menu->setObjectName(QStringLiteral("effectMenu"));
    ui::styleMenu(menu);
    menu->addAction(QStringLiteral("Reset parameters"), this, [this, index] {
        const QJsonObject current = m_clips.first().clip.value(QStringLiteral("effects")).toArray().at(index).toObject();
        QJsonObject cleared;
        const QJsonObject values = current.value(QStringLiteral("params")).toObject();
        for (auto it = values.constBegin(); it != values.constEnd(); ++it) cleared.insert(it.key(), QJsonValue());
        for (const QJsonValue &param : m_effectDefs.value(current.value(QStringLiteral("fx")).toString()).value(QStringLiteral("params")).toArray())
            cleared.insert(param.toObject().value(QStringLiteral("name")).toString(), QJsonValue());
        if (cleared.isEmpty()) return;
        emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-set")}, {QStringLiteral("clip"), m_clips.first().id},
                                       {QStringLiteral("index"), index}, {QStringLiteral("params"), cleared}}});
    })->setObjectName(QStringLiteral("effectReset"));
    menu->addAction(QStringLiteral("Ask Kerf about this effect"), this, [this, name] {
        askAbout(QStringLiteral("@%1 %2").arg(displayName(m_clips.first()), name));
    })->setObjectName(QStringLiteral("effectAsk"));
    menu->addSeparator();
    QAction *remove = menu->addAction(QStringLiteral("Remove"), this, [this, index] {
        emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-remove")}, {QStringLiteral("clip"), m_clips.first().id},
                                       {QStringLiteral("index"), index}}});
    });
    remove->setObjectName(QStringLiteral("effectRemove"));
    ui::setMenuRole(remove, ui::MenuRole::Warn);
    connect(more, &QAbstractButton::clicked, this, [more, menu] { ui::popupMenu(menu, more); });
    m_refreshers.append([this, index, enabled, title, params] {
        const QJsonObject current = m_clips.first().clip.value(QStringLiteral("effects")).toArray().at(index).toObject();
        const bool on = current.value(QStringLiteral("enabled")).toBool(true);
        {
            const QSignalBlocker blocker(enabled);
            enabled->setChecked(on);
        }
        // Switched off: its name and values in the secondary ink, which
        // still reads; they still edit.
        title->setStyleSheet(QStringLiteral("QLabel { color: %1; background: transparent; %2 }").arg(css(on ? tokens::ink : tokens::ink2), inspectorFontCss(tokens::control)));
        title->setProperty("off", !on);
        for (ScrubField *field : params->findChildren<ScrubField *>()) field->setDimmed(!on);
        for (InspectorSlider *slider : params->findChildren<InspectorSlider *>()) slider->setDimmed(!on);
        for (QLabel *hex : params->findChildren<QLabel *>(QStringLiteral("colorHex"))) {
            hex->setProperty("off", !on);
            hex->setStyleSheet(QStringLiteral("QLabel { color: %1; background: transparent; %2 }").arg(css(on ? tokens::ink : tokens::ink2), inspectorFontCss(tokens::timecode)));
        }
    });
    m_effectBoxes.append(box);
    into->addWidget(box);
}

// ------------------------------------------------------------- drag & drop

int InspectorPanel::effectIndexAt(int y) const {
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
    m_dropLine->setGeometry(InspectorRow::kPadLeft, top.y() - 1, m_content->width() - InspectorRow::kPadLeft - InspectorRow::kPadRight, 2);
    m_dropLine->raise();
    m_dropLine->show();
}

void InspectorPanel::dragEnterEvent(QDragEnterEvent *event) {
    if (m_mode == Mode::Clip && m_pane == Pane::Video && event->mimeData()->hasFormat(QString::fromLatin1(kFxMime))) event->acceptProposedAction();
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
    const QString id = QString::fromUtf8(event->mimeData()->data(QString::fromLatin1(kFxMime)));
    const int index = effectIndexAt(m_content->mapFrom(this, event->position().toPoint()).y());
    showDropLine(-1);
    if (id.isEmpty()) return;
    QJsonObject op{{QStringLiteral("op"), QStringLiteral("fx-add")}, {QStringLiteral("clip"), m_clips.first().id}, {QStringLiteral("fx"), id}};
    if (index < m_effectBoxes.size()) op.insert(QStringLiteral("index"), index);
    m_revealEffect = index;
    emitOps(QJsonArray{op});
    event->acceptProposedAction();
}

ScrubField *InspectorPanel::newScrubField() {
    auto *field = new ScrubField;
    connect(field, &ScrubField::doneEditing, this, &InspectorPanel::focusReturnRequested);
    return field;
}

void InspectorPanel::keyPressEvent(QKeyEvent *event) {
    // Esc that nothing in the Inspector took hands the keys back to the
    // timeline, so the transport works again.
    if (event->key() == Qt::Key_Escape && event->modifiers() == Qt::NoModifier) {
        emit focusReturnRequested();
        event->accept();
        return;
    }
    QWidget::keyPressEvent(event);
}

bool InspectorPanel::eventFilter(QObject *watched, QEvent *event) {
    if (watched == m_scroll && event->type() == QEvent::Resize) updateFade();
    if (watched == m_content && (event->type() == QEvent::Resize || event->type() == QEvent::LayoutRequest)) updateFade();
    auto *widget = qobject_cast<QWidget *>(watched);
    // An Info value keeps its ellipsis as its row narrows.
    if (auto *label = qobject_cast<QLabel *>(watched); label && label->objectName() == QStringLiteral("infoValue") && event->type() == QEvent::Resize) {
        const QString full = label->property("full").toString();
        const QString shown = label->fontMetrics().elidedText(full, Qt::ElideRight, std::max(0, label->width()));
        if (shown != label->text()) label->setText(shown);
    }
    if (widget && widget->objectName() == QStringLiteral("effectHeader")) {
        if (event->type() == QEvent::KeyPress) {
            const int code = static_cast<QKeyEvent *>(event)->key();
            if ((code == Qt::Key_Delete || code == Qt::Key_Backspace) && m_mode == Mode::Clip) {
                emitOps(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-remove")}, {QStringLiteral("clip"), m_clips.first().id},
                                               {QStringLiteral("index"), widget->property("index").toInt()}}});
                return true;
            }
        }
        // A click on the effect's row (not its box or buttons) folds it.
        if (event->type() == QEvent::MouseButtonRelease && static_cast<QMouseEvent *>(event)->button() == Qt::LeftButton) {
            if (auto *chevron = widget->findChild<GlyphButton *>(QStringLiteral("effectChevron"))) chevron->click();
            return true;
        }
        if (event->type() == QEvent::Paint && widget->hasFocus()) {
            QPainter painter(widget);
            painter.setRenderHint(QPainter::Antialiasing);
            ui::paintFocusRing(painter, QRectF(widget->rect()).adjusted(3, 2, -3, -2), tokens::radius::control);
        }
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
