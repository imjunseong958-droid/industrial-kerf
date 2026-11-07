#include "comment_row.h"

#include "components.h"
#include "design_tokens.h"
#include "icons.h"

#include <QEnterEvent>
#include <QFontMetricsF>
#include <QFontMetrics>
#include <QHBoxLayout>
#include <QTextDocumentFragment>
#include <QLabel>
#include <QLocale>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QRegularExpression>
#include <QSignalBlocker>
#include <QTimer>
#include <QVBoxLayout>
#include <QVariantAnimation>

#include <algorithm>
#include <cmath>
#include <limits>
#include <utility>

namespace kerf {

using namespace tokens;

// ------------------------------------------------------------------ text helpers

namespace review {

namespace {

int frameRate(double fps) { return std::max(1, int(std::lround(std::max(1.0, fps)))); }

QString colorName(QRgb rgb) { return QColor::fromRgba(rgb).name(QColor::HexRgb); }

} // namespace

QString timecode(double seconds, double fps) {
    if (!std::isfinite(seconds)) return {};
    const int rate = frameRate(fps);
    const qint64 frames = std::llround(std::max(0.0, seconds) * std::max(1.0, fps));
    const qint64 f = frames % rate;
    qint64 whole = frames / rate;
    const qint64 s = whole % 60;
    whole /= 60;
    const qint64 m = whole % 60;
    const qint64 h = whole / 60;
    return QStringLiteral("%1:%2:%3:%4")
        .arg(h, 2, 10, QLatin1Char('0'))
        .arg(m, 2, 10, QLatin1Char('0'))
        .arg(s, 2, 10, QLatin1Char('0'))
        .arg(f, 2, 10, QLatin1Char('0'));
}

QString lengthText(double seconds, double fps) {
    const int rate = frameRate(fps);
    const qint64 frames = std::llround(std::max(0.0, seconds) * std::max(1.0, fps));
    const qint64 whole = frames / rate;
    const qint64 rest = frames % rate;
    if (whole == 0) return QStringLiteral("%1f").arg(rest);
    return QStringLiteral("%1s%2f").arg(whole).arg(rest, 2, 10, QLatin1Char('0'));
}

QString markText(double at, double duration, double fps) {
    if (!std::isfinite(at)) return {};
    const QString start = timecode(at, fps);
    if (!(duration > 0.0)) return start;
    return QStringLiteral("%1 · %2").arg(start, lengthText(duration, fps));
}

QString signedFrames(qint64 frames) {
    if (frames == 0) return QStringLiteral("0f");
    return QStringLiteral("%1%2f").arg(frames < 0 ? QStringLiteral("−") : QStringLiteral("+")).arg(std::abs(frames));
}

QString age(const QDateTime &then, const QDateTime &now) {
    if (!then.isValid()) return {};
    const qint64 seconds = then.secsTo(now);
    if (seconds < 60) return QStringLiteral("now");
    if (seconds < 3600) return QStringLiteral("%1 min").arg(seconds / 60);
    if (seconds < 86400) return QStringLiteral("%1 h").arg(seconds / 3600);
    const QDate day = then.toLocalTime().date();
    if (seconds < 7 * 86400) return QLocale::c().dayName(day.dayOfWeek(), QLocale::ShortFormat);
    if (day.year() == now.toLocalTime().date().year()) return QLocale::c().toString(day, QStringLiteral("d MMM"));
    return QLocale::c().toString(day, QStringLiteral("d MMM yyyy"));
}

QString ageTip(qint64 revision, const QDateTime &created) {
    if (!created.isValid()) return revision >= 0 ? QStringLiteral("rev %1").arg(revision) : QString();
    const QDateTime local = created.toLocalTime();
    const QString when = QLocale::c().toString(local, QStringLiteral("HH:mm, d MMM"));
    return revision >= 0 ? QStringLiteral("rev %1 · %2").arg(revision).arg(when) : when;
}

double parseTimecode(const QString &text, double fps) {
    static const QRegularExpression pattern(QStringLiteral("^(\\d{2}):(\\d{2}):(\\d{2})[:;](\\d{2})$"));
    const QRegularExpressionMatch match = pattern.match(text.trimmed());
    if (!match.hasMatch()) return std::numeric_limits<double>::quiet_NaN();
    // The inverse of timecode(): whole frames of the rounded rate, at fps.
    const qint64 whole = match.captured(1).toLongLong() * 3600 + match.captured(2).toLongLong() * 60 +
                         match.captured(3).toLongLong();
    const qint64 frames = whole * frameRate(fps) + match.captured(4).toLongLong();
    return double(frames) / std::max(1.0, fps);
}

// A word too long for any line a comment has, such as a file's name, may break anywhere: a QLabel
// breaks only at spaces, and cuts off the rest. Applied to text not yet escaped.
QString breakingLongWords(const QString &text) {
    constexpr int kLongWord = 24;
    static const QRegularExpression longWord(QStringLiteral("\\S{%1,}").arg(kLongWord));
    QString out;
    qsizetype from = 0;
    for (auto it = longWord.globalMatch(text); it.hasNext();) {
        const QRegularExpressionMatch match = it.next();
        out += text.mid(from, match.capturedStart() - from).toHtmlEscaped();
        // Past each character, escaped alone so that no entity is broken.
        for (const QChar c : match.captured(0)) {
            out += QString(c).toHtmlEscaped();
            if (!c.isHighSurrogate()) out += QChar(0x200B);
        }
        from = match.capturedEnd();
    }
    return out + text.mid(from).toHtmlEscaped();
}

QString richBody(const QString &body, double fps) {
    static const QRegularExpression token(
        QStringLiteral("(\\d{2}:\\d{2}:\\d{2}[:;]\\d{2})|(@[A-Za-z][A-Za-z0-9_.\\-]*)"));
    // The link colour is accent ink at the design's 85% over the card.
    const QString link = ui::mix(color(card), color(accentInk), 0.85).name(QColor::HexRgb);
    QString out;
    qsizetype from = 0;
    QRegularExpressionMatchIterator it = token.globalMatch(body);
    while (it.hasNext()) {
        const QRegularExpressionMatch match = it.next();
        out += breakingLongWords(body.mid(from, match.capturedStart() - from));
        const QString text = match.captured(0);
        if (!match.captured(1).isEmpty()) {
            const double seconds = parseTimecode(text, fps);
            out += QStringLiteral("<a href=\"tc:%1\" style=\"color:%2; text-decoration:none; font-family:'%3','%4'; "
                                  "font-size:12px;\">%5</a>")
                       .arg(QString::number(seconds, 'g', 12), link, QString::fromLatin1(monoFamily),
                            QString::fromLatin1(monoFallback), text.toHtmlEscaped());
        } else {
            out += QStringLiteral("<span style=\"color:%1; font-weight:500;\">%2</span>")
                       .arg(colorName(ink), text.toHtmlEscaped());
        }
        from = match.capturedEnd();
    }
    out += breakingLongWords(body.mid(from));
    out.replace(QLatin1Char('\n'), QStringLiteral("<br>"));
    return QStringLiteral("<div style=\"line-height:20px;\">%1</div>").arg(out);
}

QStringList mentions(const QString &text) {
    static const QRegularExpression pattern(QStringLiteral("(?:^|[^A-Za-z0-9_])@([A-Za-z][A-Za-z0-9_.\\-]*)"));
    QStringList out;
    QRegularExpressionMatchIterator it = pattern.globalMatch(text);
    while (it.hasNext()) out << it.next().captured(1);
    return out;
}

bool addressesKerf(const QString &text) {
    const QStringList names = mentions(text);
    if (names.isEmpty()) return true;
    return std::any_of(names.begin(), names.end(),
                       [](const QString &name) { return name.compare(QLatin1String("Kerf"), Qt::CaseInsensitive) == 0; });
}

} // namespace review

namespace {

QLabel *makeLabel(const TypeStyle &style, QRgb ink, QWidget *parent = nullptr) {
    auto *label = new QLabel(parent);
    label->setFont(tokens::font(style));
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, color(ink));
    palette.setColor(QPalette::Text, color(ink));
    label->setPalette(palette);
    label->setFixedHeight(style.line);
    return label;
}

void setInk(QLabel *label, QRgb ink) {
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, color(ink));
    palette.setColor(QPalette::Text, color(ink));
    label->setPalette(palette);
}

// A widget that holds `child` with `top` points above it, hidden with it.
QWidget *holder(QWidget *child, int top, int left = 0, int right = 0) {
    auto *box = new QWidget;
    auto *layout = new QVBoxLayout(box);
    layout->setContentsMargins(left, top, right, 0);
    layout->setSpacing(0);
    layout->addWidget(child);
    return box;
}

// A 14 pt glyph in ink3 (the drawing flag).
class GlyphFlag final : public QWidget {
public:
    GlyphFlag(const QString &icon, QWidget *parent = nullptr) : QWidget(parent), m_icon(icon) { setFixedSize(14, 20); }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing);
        icons::paint(p, m_icon, QRectF(rect()), color(ink3), 14);
    }

private:
    QString m_icon;
};

// The hover actions' raised pill.
class ActionsBar final : public QWidget {
public:
    using QWidget::QWidget;

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing);
        p.setPen(Qt::NoPen);
        p.setBrush(color(raised));
        p.drawRoundedRect(QRectF(rect()), radius::control, radius::control);
    }
};

} // namespace

// ------------------------------------------------------------------ TimecodeLink

TimecodeLink::TimecodeLink(QWidget *parent) : QAbstractButton(parent) {
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    setFont(tokens::font(timecode));
    setFixedHeight(timecode.line);
}

void TimecodeLink::setMark(double at, double duration, double fps) {
    m_at = at;
    m_duration = duration;
    setText(review::markText(at, duration, fps));
    setAccessibleName(text());
    updateGeometry();
    update();
}

QSize TimecodeLink::sizeHint() const {
    return QSize(ui::snapWidth(QFontMetricsF(font()), text()), timecode.line);
}

void TimecodeLink::paintEvent(QPaintEvent *) {
    QPainter p(this);
    const bool lit = underMouse() || hasFocus();
    p.setOpacity(lit ? 1.0 : 0.85);
    p.setFont(font());
    p.setPen(color(accentInk));
    const QFontMetricsF metrics(font());
    const qreal baseline = (height() - metrics.height()) / 2 + metrics.ascent();
    p.drawText(QPointF(0, baseline), text());
    if (lit) {
        QPen pen(color(accentInk), 1, Qt::DotLine);
        p.setPen(pen);
        const qreal y = std::min<qreal>(height() - 0.5, baseline + 3 - 0.5);
        p.drawLine(QPointF(0, y), QPointF(metrics.horizontalAdvance(text()), y));
    }
}

// ------------------------------------------------------------------ ResolveRing

ResolveRing::ResolveRing(QWidget *parent) : QAbstractButton(parent) {
    setFixedSize(24, 24);
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    ui::setTip(this, QStringLiteral("Resolve"));
    setAccessibleName(QStringLiteral("Resolve"));
}

void ResolveRing::setResolved(bool resolved) {
    m_resolved = resolved;
    const QString name = resolved ? QStringLiteral("Reopen") : QStringLiteral("Resolve");
    ui::setTip(this, name);
    setAccessibleName(name);
    update();
}

void ResolveRing::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const bool hover = underMouse() && isEnabled();
    if (hover) {
        p.setPen(Qt::NoPen);
        p.setBrush(color(raised));
        p.drawRoundedRect(QRectF(rect()), radius::control, radius::control);
    }
    const QColor ink = !isEnabled() ? color(dis) : hover ? color(tokens::ink) : m_resolved ? color(ink2) : color(ink3);
    icons::paint(p, m_resolved ? QStringLiteral("g-ringdone") : QStringLiteral("g-ring"), QRectF(rect()), ink, 14);
    if (hasFocus()) ui::paintFocusRing(p, QRectF(rect()), radius::control);
}

// ------------------------------------------------------------------ TagButton

TagButton::TagButton(const QString &text, Style style, QWidget *parent) : QAbstractButton(parent), m_style(style) {
    setText(text);
    setFont(tokens::font(meta));
    setFixedHeight(meta.line);
    const bool clickable = style == Style::Tag || style == Style::Blocking;
    setEnabled(true);
    setCursor(clickable ? Qt::PointingHandCursor : Qt::ArrowCursor);
    setFocusPolicy(clickable ? Qt::TabFocus : Qt::NoFocus);
    if (!clickable) setAttribute(Qt::WA_TransparentForMouseEvents);
    if (style == Style::Blocking) ui::setTip(this, QStringLiteral("Blocks Render master"));
}

QColor TagButton::textColor() const {
    switch (m_style) {
    case Style::Failed: return color(neg);
    case Style::Blocking: return color(ink2);
    case Style::Accepted: return color(ink3);
    case Style::Tag: return underMouse() ? color(ink) : color(ink3);
    }
    return color(ink3);
}

QSize TagButton::sizeHint() const {
    const bool dot = m_style == Style::Blocking || m_style == Style::Failed;
    return QSize((dot ? 6 + 4 : 0) + ui::snapWidth(QFontMetricsF(font()), text()), meta.line);
}

void TagButton::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    qreal x = 0;
    if (m_style == Style::Blocking || m_style == Style::Failed) {
        p.setPen(Qt::NoPen);
        p.setBrush(color(neg));
        p.drawEllipse(QRectF(0, (height() - 6) / 2.0, 6, 6));
        x = 10;
    }
    p.setFont(font());
    p.setPen(textColor());
    const QFontMetricsF metrics(font());
    p.drawText(QPointF(x, (height() - metrics.height()) / 2 + metrics.ascent()), text());
    if (hasFocus()) ui::paintFocusRing(p, QRectF(rect()), 2);
}

// ------------------------------------------------------------------ ChatExpander

ChatExpander::ChatExpander(QWidget *parent) : QAbstractButton(parent) {
    setCheckable(true);
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    setFont(tokens::font(control));
    setFixedHeight(20);
    m_rotate = new QVariantAnimation(this);
    m_rotate->setEasingCurve(easing());
    connect(m_rotate, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
        m_turn = value.toReal();
        update();
    });
    connect(this, &QAbstractButton::toggled, this, [this](bool on) {
        m_rotate->stop();
        const int ms = ui::motionDuration(motion::fade);
        if (ms <= 0 || !isVisible()) {
            m_turn = on ? 1 : 0;
            update();
            return;
        }
        m_rotate->setDuration(ms);
        m_rotate->setStartValue(m_turn);
        m_rotate->setEndValue(on ? 1.0 : 0.0);
        m_rotate->start();
    });
}

void ChatExpander::setOpenQuietly(bool open) {
    const QSignalBlocker blocker(this);
    setChecked(open);
    m_rotate->stop();
    m_turn = open ? 1 : 0;
    update();
}

void ChatExpander::setSmall(bool small) {
    m_small = small;
    setFont(tokens::font(small ? meta : control));
    setFixedHeight(small ? meta.line : 20);
    updateGeometry();
}

void ChatExpander::setChevron(bool chevron) {
    m_chevron = chevron;
    setCheckable(chevron);
    updateGeometry();
    update();
}

QSize ChatExpander::sizeHint() const {
    const int text = ui::snapWidth(QFontMetricsF(font()), this->text());
    return QSize(text + (m_chevron ? 4 + 14 : 0), m_small ? meta.line : 20);
}

void ChatExpander::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const QColor ink = underMouse() ? color(tokens::ink) : color(ink2);
    p.setFont(font());
    p.setPen(ink);
    const QFontMetricsF metrics(font());
    p.drawText(QPointF(0, (height() - metrics.height()) / 2 + metrics.ascent()), text());
    if (m_chevron) {
        const qreal x = ui::snapWidth(metrics, text()) + 4;
        const QRectF box(x, (height() - 14) / 2.0, 14, 14);
        p.save();
        p.translate(box.center());
        p.rotate(180.0 * m_turn);
        p.translate(-box.center());
        icons::paint(p, QStringLiteral("g-chev"), box, ink, 14);
        p.restore();
    }
    if (hasFocus()) ui::paintFocusRing(p, QRectF(rect()), 2);
}

// ------------------------------------------------------------------ SuggestionButton

SuggestionButton::SuggestionButton(const QString &text, QWidget *parent) : QAbstractButton(parent), m_request(text) {
    setText(text);
    setFont(tokens::font(control));
    setFixedHeight(28);
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
}

QSize SuggestionButton::sizeHint() const {
    return QSize(8 + 14 + 8 + ui::snapWidth(QFontMetricsF(font()), text()) + 8, 28);
}

QRect SuggestionButton::textRect() const { return QRect(8 + 14 + 8, 0, width() - 8 - 14 - 8 - 8, height()); }

void SuggestionButton::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    if (underMouse() || isDown()) {
        p.setPen(Qt::NoPen);
        p.setBrush(color(raised));
        p.drawRoundedRect(QRectF(rect()), radius::control, radius::control);
    }
    icons::paint(p, QStringLiteral("g-cdr"), QRectF(8, (height() - 14) / 2.0, 14, 14), color(ink3), 14);
    // Timecodes in the words read in mono ink2, as the welcome writes them.
    static const QRegularExpression tc(QStringLiteral("\\d{2}:\\d{2}:\\d{2}[:;]\\d{2}"));
    const QRect area = textRect();
    const QFont uiFont = font();
    const QFont mono = tokens::font(timecode);
    const QFontMetricsF uiMetrics(uiFont);
    qreal x = area.left();
    const qreal baseline = (height() - uiMetrics.height()) / 2 + uiMetrics.ascent();
    const QString words = text();
    qsizetype from = 0;
    const auto draw = [&](const QString &part, const QFont &face, const QColor &ink) {
        if (part.isEmpty() || x >= area.right()) return;
        const QFontMetricsF metrics(face);
        const QString shown = metrics.elidedText(part, Qt::ElideRight, area.right() - x);
        p.setFont(face);
        p.setPen(ink);
        p.drawText(QPointF(x, baseline), shown);
        x += metrics.horizontalAdvance(shown);
    };
    QRegularExpressionMatchIterator it = tc.globalMatch(words);
    while (it.hasNext()) {
        const QRegularExpressionMatch match = it.next();
        draw(words.mid(from, match.capturedStart() - from), uiFont, color(ink));
        draw(match.captured(0), mono, color(ink2));
        from = match.capturedEnd();
    }
    draw(words.mid(from), uiFont, color(ink));
    if (hasFocus()) ui::paintFocusRing(p, QRectF(rect()), radius::control);
}

// ------------------------------------------------------------------ DraftDots

DraftDots::DraftDots(QWidget *parent) : QWidget(parent) {
    setFixedSize(sizeHint());
    m_timer = new QTimer(this);
    m_timer->setInterval(320);
    connect(m_timer, &QTimer::timeout, this, [this] {
        m_phase = (m_phase + 1) % 3;
        update();
    });
}

void DraftDots::showEvent(QShowEvent *) {
    if (!ui::reducedMotion()) m_timer->start();
}

void DraftDots::hideEvent(QHideEvent *) { m_timer->stop(); }

void DraftDots::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    p.setPen(Qt::NoPen);
    const bool moving = m_timer->isActive();
    for (int i = 0; i < 3; ++i) {
        QColor dot = color(ink3);
        if (moving && i != m_phase) dot.setAlphaF(0.45);
        p.setBrush(dot);
        p.drawEllipse(QRectF(2 + i * 6, (height() - 3) / 2.0, 3, 3));
    }
}

// ------------------------------------------------------------------ DraftSkeleton

DraftSkeleton::DraftSkeleton(QWidget *parent) : QWidget(parent) {
    setFixedHeight(8 + 8 + 8);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
}

void DraftSkeleton::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    p.setPen(Qt::NoPen);
    p.setBrush(color(raised));
    p.drawRoundedRect(QRectF(0, 0, width() * 0.88, 8), radius::small, radius::small);
    p.drawRoundedRect(QRectF(0, 16, width() * 0.56, 8), radius::small, radius::small);
}

// ------------------------------------------------------------------ CommentRow

CommentRow::CommentRow(Kind kind, const QString &text, const QDateTime &time, QWidget *parent)
    : QWidget(parent), m_kind(kind), m_text(text) {
    setObjectName(QStringLiteral("commentRow"));
    setAttribute(Qt::WA_Hover);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
    build();
    switch (kind) {
    case Kind::User:
        setAuthor(QStringLiteral("person"), QString());
        setName(QStringLiteral("You"));
        break;
    case Kind::Kerf:
    case Kind::Drafting:
        setAuthor(QStringLiteral("kerf"), QString());
        setName(QStringLiteral("Kerf"));
        break;
    case Kind::Error:
        setAuthor(QStringLiteral("kerf"), QString());
        setName(QStringLiteral("Kerf"));
        setError(true);
        break;
    case Kind::Record: break;
    }
    if (kind == Kind::Drafting) {
        m_kind = Kind::Kerf;
        setDrafting(true);
    }
    setTime(time);
    updateBody();
}

CommentRow::~CommentRow() = default;

void CommentRow::build() {
    auto *outer = new QVBoxLayout(this);
    outer->setContentsMargins(0, kBleedY, 0, kBleedY);
    outer->setSpacing(0);
    auto *top = new QHBoxLayout;
    top->setContentsMargins(kBleedX, 0, kBleedX, 0);
    top->setSpacing(kGap);
    m_avatar = new ui::Avatar(ui::Avatar::Kind::Person);
    m_avatar->setFixedSize(kAvatar, kAvatar);
    top->addWidget(m_avatar, 0, Qt::AlignTop);
    m_bodyLayout = new QVBoxLayout;
    m_bodyLayout->setContentsMargins(0, 0, 0, 0);
    m_bodyLayout->setSpacing(0);
    top->addLayout(m_bodyLayout, 1);
    outer->addLayout(top);
    m_wideLayout = new QVBoxLayout;
    m_wideLayout->setContentsMargins(0, 0, 0, 0);
    m_wideLayout->setSpacing(0);
    outer->addLayout(m_wideLayout);

    // The name line, 20 tall, gap 6.
    m_nameLine = new QWidget;
    m_nameLine->setFixedHeight(20);
    m_nameLayout = new QHBoxLayout(m_nameLine);
    m_nameLayout->setContentsMargins(0, 0, 0, 0);
    m_nameLayout->setSpacing(6);
    m_nameLabel = makeLabel(bodyName, ink);
    m_nameLabel->setObjectName(QStringLiteral("commentName"));
    m_roleLabel = makeLabel(control, ink3);
    m_roleLabel->hide();
    m_tc = new TimecodeLink;
    m_tc->hide();
    connect(m_tc, &QAbstractButton::clicked, this, [this] { emit seekRequested(m_tc->at()); });
    m_errorLabel = makeLabel(meta, neg);
    m_errorLabel->setText(QStringLiteral("Couldn't edit"));
    m_errorLabel->hide();
    m_drawingFlag = new GlyphFlag(QStringLiteral("g-annot"));
    ui::setTip(m_drawingFlag, QStringLiteral("Has a drawing"));
    m_drawingFlag->hide();
    m_timeLabel = makeLabel(meta, ink3);
    m_dots = new DraftDots;
    m_dots->hide();
    m_metaLabel = makeLabel(meta, ink3);
    m_metaLabel->hide();
    m_metaLabel->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Fixed);
    m_stop = new ui::GhostButton(QStringLiteral("Stop"));
    m_stop->setControlHeight(20);
    m_stop->setTip(QStringLiteral("Stop"), QStringLiteral("Esc"));
    m_stop->hide();
    connect(m_stop, &QAbstractButton::clicked, this, [this] {
        if (m_stopped) emit askAgainClicked();
        else emit stopClicked();
    });
    m_ringRoom = new QWidget;
    m_ringRoom->setFixedSize(24 - 5, 20);
    m_ringRoom->hide();
    for (QWidget *part : std::initializer_list<QWidget *>{m_nameLabel, m_roleLabel, m_tc, m_errorLabel, m_drawingFlag,
                                                          m_timeLabel, m_dots})
        m_nameLayout->addWidget(part, 0, Qt::AlignVCenter);
    m_nameLayout->addWidget(m_metaLabel, 1, Qt::AlignVCenter);
    m_nameLayout->addStretch(0);
    m_nameLayout->addWidget(m_stop, 0, Qt::AlignVCenter);
    m_nameLayout->addWidget(m_ringRoom);
    m_bodyLayout->addWidget(m_nameLine);

    // The text, 2 under the name line.
    m_body = new QLabel;
    m_body->setObjectName(QStringLiteral("commentBody"));
    m_body->setFont(tokens::font(tokens::reading));
    m_body->setWordWrap(true);
    m_body->setTextFormat(Qt::RichText);
    m_body->setTextInteractionFlags(Qt::LinksAccessibleByMouse | Qt::LinksAccessibleByKeyboard);
    m_body->setOpenExternalLinks(false);
    m_body->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    setInk(m_body, ink);
    connect(m_body, &QLabel::linkActivated, this, [this](const QString &link) {
        if (link.startsWith(QLatin1String("tc:"))) {
            bool ok = false;
            const double seconds = link.mid(3).toDouble(&ok);
            if (ok && std::isfinite(seconds)) emit seekRequested(seconds);
        }
    });
    m_bodyLayout->addWidget(holder(m_body, 2));

    // The tags row, 4 under the text.
    m_tagsRow = new QWidget;
    m_tagsRow->setMinimumHeight(meta.line);
    m_tagsLayout = new QHBoxLayout(m_tagsRow);
    m_tagsLayout->setContentsMargins(0, 0, 0, 0);
    m_tagsLayout->setSpacing(8);
    m_tagsLayout->addStretch(1);
    QWidget *tagsHolder = holder(m_tagsRow, 4);
    tagsHolder->setObjectName(QStringLiteral("tagsHolder"));
    m_bodyLayout->addWidget(tagsHolder);
    tagsHolder->hide();

    // Steps, 4 under the tags row.
    m_stepsList = new QWidget;
    m_stepsLayout = new QVBoxLayout(m_stepsList);
    m_stepsLayout->setContentsMargins(0, 4, 0, 0);
    m_stepsLayout->setSpacing(0);
    m_stepsList->hide();
    m_bodyLayout->addWidget(m_stepsList);

    // The skeleton, 8 under the name line.
    m_skeleton = new DraftSkeleton;
    QWidget *skeletonHolder = holder(m_skeleton, 8);
    skeletonHolder->setObjectName(QStringLiteral("skeletonHolder"));
    skeletonHolder->hide();
    m_bodyLayout->addWidget(skeletonHolder);

    // Hover actions, over the row.
    m_actions = new ActionsBar(this);
    auto *actions = new QHBoxLayout(m_actions);
    actions->setContentsMargins(2, 2, 2, 2);
    actions->setSpacing(4);
    const auto action = [actions](const QString &icon, const QString &tip) {
        auto *button = new ui::IconButton(icon, ui::IconButton::Size::S24);
        button->setIconPx(14);
        button->setTip(tip);
        button->setAccessibleName(tip);
        actions->addWidget(button);
        return button;
    };
    m_replyAction = action(QStringLiteral("g-reply"), QStringLiteral("Reply"));
    m_resolveAction = action(QStringLiteral("g-check"), QStringLiteral("Resolve"));
    m_moreAction = action(QStringLiteral("g-more"), QStringLiteral("More"));
    connect(m_replyAction, &QAbstractButton::clicked, this, &CommentRow::replyClicked);
    connect(m_resolveAction, &QAbstractButton::clicked, this, &CommentRow::ringClicked);
    connect(m_moreAction, &QAbstractButton::clicked, this, &CommentRow::moreClicked);
    m_actions->hide();

    m_ring = new ResolveRing(this);
    m_ring->hide();
    connect(m_ring, &QAbstractButton::clicked, this, &CommentRow::ringClicked);

    m_fade = new QVariantAnimation(this);
    m_fade->setEasingCurve(easing());
    connect(m_fade, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
        m_hover = value.toReal();
        update();
    });
    m_flashTimer = new QTimer(this);
    m_flashTimer->setSingleShot(true);
    m_flashTimer->setInterval(600);
    connect(m_flashTimer, &QTimer::timeout, this, [this] {
        m_flashing = false;
        update();
    });
}

void CommentRow::setReply(bool reply) {
    m_reply = reply;
    placeOverlays();
}

void CommentRow::setAuthor(const QString &kind, const QString &initials) {
    m_avatar->setKind(ui::Avatar::kindFor(kind));
    m_avatar->setInitials(initials);
}

QString CommentRow::name() const { return m_nameLabel->text(); }

void CommentRow::setName(const QString &name) {
    m_name = name;
    if (!isDrafting()) m_nameLabel->setText(name);
    setAccessibleName(name);
    updateNameLine();
}

void CommentRow::setRole(const QString &role) {
    m_roleLabel->setText(role);
    // Kerf at work says only that it drafts (no role).
    m_roleLabel->setVisible(!role.isEmpty() && !isDrafting());
    updateNameLine();
}

void CommentRow::setMark(double at, double duration, double fps) {
    m_fps = fps;
    const bool shown = std::isfinite(at);
    if (shown) m_tc->setMark(at, duration, fps);
    m_tc->setVisible(shown);
    ui::setTip(m_tc, shown ? QStringLiteral("Go to %1").arg(review::timecode(at, fps)) : QString());
    updateNameLine();
}

void CommentRow::setTime(const QDateTime &time, qint64 revision) {
    m_time = time;
    m_revision = revision;
    refreshTime(QDateTime::currentDateTime());
    ui::setTip(m_timeLabel, review::ageTip(revision, time));
}

void CommentRow::refreshTime(const QDateTime &now) {
    m_timeLabel->setText(review::age(m_time, now));
    m_timeLabel->setVisible(!isDrafting() && m_time.isValid());
    updateNameLine();
}

void CommentRow::setError(bool error) {
    m_error = error;
    m_errorLabel->setVisible(error);
    updateNameLine();
}

void CommentRow::setDrawing(bool drawing) {
    m_drawingFlag->setVisible(drawing);
    updateNameLine();
}

void CommentRow::setText(const QString &text) {
    m_text = text;
    updateBody();
}

void CommentRow::appendText(const QString &text) {
    m_text += text;
    updateBody();
}

void CommentRow::setFps(double fps) {
    m_fps = fps;
    updateBody();
}

void CommentRow::setMuted(bool muted) {
    m_muted = muted;
    setInk(m_body, muted ? ink2 : ink);
}

void CommentRow::updateBody() {
    m_body->setText(review::richBody(m_text, m_fps));
    m_body->parentWidget()->setVisible(!m_text.isEmpty());
    if (QWidget *skeleton = m_skeleton ? m_skeleton->parentWidget() : nullptr)
        skeleton->setVisible(isDrafting() && !m_stopped && m_text.isEmpty());
}

void CommentRow::setRing(bool shown, bool resolved) {
    m_ring->setResolved(resolved);
    m_ring->setVisible(shown && m_look == Look::Note);
    m_ringRoom->setVisible(shown);
    m_resolveAction->setTip(resolved ? QStringLiteral("Reopen") : QStringLiteral("Resolve"));
    placeOverlays();
}

TagButton *CommentRow::addTag(const QString &text, TagButton::Style style, const QString &tag) {
    auto *button = new TagButton(text, style);
    button->setTag(tag);
    connect(button, &QAbstractButton::clicked, this, [this, button] {
        if (!button->tag().isEmpty()) emit tagClicked(button->tag());
    });
    m_tagsLayout->insertWidget(m_tagsLayout->count() - 1, button);
    m_tags.append(button);
    m_tagsRow->parentWidget()->show();
    return button;
}

ChatExpander *CommentRow::setReplyFold(int replies, bool open) {
    if (replies <= 0) {
        if (m_replyFold) m_replyFold->hide();
        return nullptr;
    }
    if (!m_replyFold) {
        m_replyFold = new ChatExpander;
        m_replyFold->setSmall(true);
        m_tagsLayout->insertWidget(m_tagsLayout->count() - 1, m_replyFold);
        connect(m_replyFold, &QAbstractButton::toggled, this, &CommentRow::foldToggled);
    }
    m_replyFold->setText(replies == 1 ? QStringLiteral("1 reply") : QStringLiteral("%1 replies").arg(replies));
    m_replyFold->setOpenQuietly(open);
    m_replyFold->show();
    m_tagsRow->parentWidget()->show();
    return m_replyFold;
}

ChatExpander *CommentRow::addLink(const QString &text) {
    auto *link = new ChatExpander;
    link->setSmall(true);
    link->setChevron(false);
    link->setText(text);
    m_tagsLayout->setSpacing(m_links.isEmpty() && m_tags.isEmpty() ? 12 : m_tagsLayout->spacing());
    m_tagsLayout->insertWidget(m_tagsLayout->count() - 1, link);
    m_links.append(link);
    m_tagsRow->parentWidget()->show();
    return link;
}

void CommentRow::addReply(const QString &text) {
    if (text.trimmed().isEmpty()) return;
    m_steps.append(text);
    updateSteps();
}

void CommentRow::setSteps(const QStringList &steps) {
    m_steps = steps;
    updateSteps();
}

void CommentRow::updateSteps() {
    if (m_steps.isEmpty()) {
        if (m_stepsLink) m_stepsLink->hide();
        m_stepsList->hide();
        return;
    }
    if (!m_stepsLink) {
        m_stepsLink = new ChatExpander;
        m_tagsLayout->insertWidget(0, m_stepsLink);
        connect(m_stepsLink, &QAbstractButton::toggled, this, [this](bool open) {
            m_stepsList->setVisible(open);
            emit stepsToggled(open);
        });
    }
    m_stepsLink->setText(m_steps.size() == 1 ? QStringLiteral("1 step") : QStringLiteral("%1 steps").arg(m_steps.size()));
    m_stepsLink->show();
    m_tagsRow->parentWidget()->show();
    while (QLayoutItem *item = m_stepsLayout->takeAt(0)) {
        delete item->widget();
        delete item;
    }
    for (int index = 0; index < m_steps.size(); ++index) {
        auto *line = new QWidget;
        auto *layout = new QHBoxLayout(line);
        layout->setContentsMargins(0, 0, 0, 0);
        layout->setSpacing(8);
        QLabel *number = makeLabel(monoSmall, ink3);
        number->setText(QString::number(index + 1));
        number->setFixedHeight(20);
        QLabel *text = makeLabel(control, ink2);
        text->setFixedHeight(QWIDGETSIZE_MAX);
        text->setMinimumHeight(20);
        text->setWordWrap(true);
        text->setTextFormat(Qt::PlainText);
        text->setText(m_steps.at(index));
        text->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
        layout->addWidget(number, 0, Qt::AlignTop);
        layout->addWidget(text, 1);
        m_stepsLayout->addWidget(line);
    }
    m_stepsList->setVisible(m_stepsLink->isChecked());
}

bool CommentRow::repliesExpanded() const { return m_stepsLink && m_stepsLink->isChecked(); }

void CommentRow::setRepliesExpanded(bool expanded) {
    if (!m_stepsLink) return;
    m_stepsLink->setChecked(expanded);
    m_stepsList->setVisible(expanded);
}

void CommentRow::setSuggestions(const QList<QPair<QString, QString>> &labelAndRequest) {
    if (!m_suggestionBox) {
        m_suggestionBox = new QWidget;
        m_suggestionBox->setObjectName(QStringLiteral("suggestions"));
        auto *layout = new QVBoxLayout(m_suggestionBox);
        // The rows' wash reaches 8 past the words, as .sug's margin 0 −8.
        layout->setContentsMargins(kBleedX + kBodyLeft - 8, 0, 0, 0);
        layout->setSpacing(0);
        m_tryAsking = makeLabel(meta, ink3);
        m_tryAsking->setText(QStringLiteral("Try asking"));
        layout->addWidget(holder(m_tryAsking, 8, 8));
        m_wideLayout->insertWidget(0, m_suggestionBox);
    }
    auto *layout = static_cast<QVBoxLayout *>(m_suggestionBox->layout());
    for (SuggestionButton *button : std::as_const(m_suggestions)) {
        layout->removeWidget(button);
        button->deleteLater();
    }
    m_suggestions.clear();
    for (const auto &[label, request] : labelAndRequest) {
        auto *button = new SuggestionButton(label);
        button->setRequest(request.isEmpty() ? label : request);
        ui::setTip(button, request.isEmpty() ? label : request);
        connect(button, &QAbstractButton::clicked, this, [this, button] { emit suggestionChosen(button->request()); });
        layout->addWidget(button);
        m_suggestions.append(button);
    }
    m_suggestionBox->setVisible(!m_suggestions.isEmpty());
}

void CommentRow::setDrafting(bool drafting) {
    m_kind = drafting ? Kind::Drafting : (m_kind == Kind::Drafting ? Kind::Kerf : m_kind);
    m_stopped = false;
    m_nameLabel->setText(drafting ? QStringLiteral("Kerf is drafting") : m_name);
    if (m_look == Look::Answer) m_nameLine->setVisible(drafting);
    m_dots->setVisible(drafting);
    m_metaLabel->setVisible(drafting && !m_metaLabel->text().isEmpty());
    m_stop->setText(QStringLiteral("Stop"));
    m_stop->setFixedSize(m_stop->sizeHint());
    m_stop->setTip(QStringLiteral("Stop"), QStringLiteral("Esc"));
    m_stop->setVisible(drafting);
    m_timeLabel->setVisible(!drafting && m_time.isValid());
    m_roleLabel->setVisible(!drafting && !m_roleLabel->text().isEmpty());
    updateBody();
    updateNameLine();
}

void CommentRow::setDraftMeta(const QString &meta) {
    m_metaLabel->setText(meta);
    ui::setTip(m_metaLabel, meta);
    m_metaLabel->setVisible(isDrafting() && !meta.isEmpty());
}

QString CommentRow::draftMeta() const { return m_metaLabel->text(); }

void CommentRow::setStopped(bool stopped) {
    m_stopped = stopped;
    if (stopped) {
        m_kind = Kind::Kerf;
        m_nameLabel->setText(QStringLiteral("Kerf · stopped"));
        m_dots->hide();
        m_metaLabel->hide();
        m_stop->setText(QStringLiteral("Ask again"));
        m_stop->setFixedSize(m_stop->sizeHint());
        m_stop->setTip(QStringLiteral("Ask again"));
        m_stop->show();
    } else {
        m_nameLabel->setText(m_name);
        m_stop->hide();
    }
    updateBody();
    updateNameLine();
}

void CommentRow::finish() {
    if (!isDrafting()) return;
    m_kind = Kind::Kerf;
    m_nameLabel->setText(m_name);
    m_dots->hide();
    m_metaLabel->hide();
    m_stop->hide();
    m_roleLabel->setVisible(!m_roleLabel->text().isEmpty());
    if (!m_time.isValid()) setTime(QDateTime::currentDateTime());
    m_timeLabel->show();
    updateBody();
    updateNameLine();
}

void CommentRow::addBodyWidget(QWidget *widget) { m_bodyLayout->addWidget(holder(widget, 8)); }

void CommentRow::addWideWidget(QWidget *widget) {
    m_wideLayout->addWidget(holder(widget, 8, kBleedX, kBleedX));
}

void CommentRow::setSelected(bool selected) {
    m_selected = selected;
    m_avatar->setSelected(selected);
    setInk(m_roleLabel, selected ? ink2 : ink3);
    setInk(m_timeLabel, selected ? ink2 : ink3);
    update();
}

void CommentRow::setCurrent(bool current) {
    m_current = current;
    update();
}

void CommentRow::flash() {
    m_flashing = true;
    m_flashTimer->start();
    update();
}

QColor CommentRow::backgroundColor() const {
    if (m_flashing) return color(hover2);
    if (m_current || m_selected) return color(on);
    QColor wash = color(raised);
    wash.setAlphaF(m_hover);
    return wash;
}

void CommentRow::setActions(bool reply, bool resolve, bool more) {
    m_replyAction->setVisible(reply);
    m_resolveAction->setVisible(resolve);
    m_moreAction->setVisible(more);
    m_hasActions = reply || resolve || more;
    m_actions->adjustSize();
    placeOverlays();
}

void CommentRow::setActionsShown(bool shown) {
    m_actions->setVisible(shown && m_hasActions);
    if (shown) m_actions->raise();
}

QRect CommentRow::bodyRect() const {
    return QRect(kBleedX + kBodyLeft, kBleedY, width() - 2 * kBleedX - kBodyLeft, height() - 2 * kBleedY);
}

void CommentRow::placeOverlays() {
    const int right = width() - kBleedX;
    m_ring->move(right + 5 - 24, kBleedY - 2);
    m_ring->raise();
    m_actions->adjustSize();
    const int x = m_reply ? right + 4 - m_actions->width() : right - 20 - m_actions->width();
    m_actions->move(x, kBleedY - 4);
}

void CommentRow::updateNameLine() {
    if (m_nameLayout) m_nameLayout->invalidate();
}

void CommentRow::setLook(Look look) {
    if (look == m_look) return;
    m_look = look;
    m_avatar->setVisible(look == Look::Note);
    if (look != Look::Note) m_ring->hide();
    m_nameLine->setVisible(look == Look::Note || (look == Look::Answer && isDrafting()));
    if (look == Look::Request) {
        m_body->setContentsMargins(14, 9, 14, 9);
    }
    fitBubble();
    update();
}

void CommentRow::fitBubble() {
    if (m_look != Look::Request) return;
    const int room = int((width() - 2 * kBleedX) * 0.82);
    const QString plain = QTextDocumentFragment::fromHtml(m_body->text()).toPlainText();
    const int wide = QFontMetrics(m_body->font()).horizontalAdvance(plain) + 28 + 2;
    const int shown = std::clamp(wide, 1, std::max(1, room));
    // Pushed over within its holder, which asks for no width of its own: the row
    // can still narrow as the conversation gives way.
    QWidget *holding = m_body->parentWidget();
    if (!holding || !holding->layout()) return;
    holding->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    QMargins margins = holding->layout()->contentsMargins();
    margins.setLeft(std::max(0, width() - 2 * kBleedX - shown));
    holding->layout()->setContentsMargins(margins);
}

void CommentRow::paintEvent(QPaintEvent *) {
    if (m_look == Look::Request) {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing);
        p.setPen(Qt::NoPen);
        p.setBrush(QColor(0x30, 0x30, 0x30));
        const QPoint at = m_body->mapTo(this, QPoint());
        p.drawRoundedRect(QRectF(at, m_body->size()), 14, 14);
        return;
    }
    const QColor fill = backgroundColor();
    if (fill.alpha() == 0) return;
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    p.setPen(Qt::NoPen);
    p.setBrush(fill);
    const QRectF area(rect());
    p.drawRoundedRect(area, radius::control, radius::control);
    if (m_current && !m_flashing) ui::paintInsetLine(p, area, radius::control, color(line));
}

void CommentRow::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    fitBubble();
    placeOverlays();
}

void CommentRow::fadeTo(qreal target) {
    m_fade->stop();
    const int ms = ui::motionDuration(motion::hover);
    if (ms <= 0) {
        m_hover = target;
        update();
        return;
    }
    m_fade->setDuration(ms);
    m_fade->setStartValue(m_hover);
    m_fade->setEndValue(target);
    m_fade->start();
}

void CommentRow::enterEvent(QEnterEvent *event) {
    QWidget::enterEvent(event);
    m_hovered = true;
    // A reply's hover belongs to the reply, not its root as well.
    fadeTo(1);
    setActionsShown(true);
    emit hoverChanged(true);
}

void CommentRow::leaveEvent(QEvent *event) {
    QWidget::leaveEvent(event);
    m_hovered = false;
    fadeTo(0);
    setActionsShown(false);
    emit hoverChanged(false);
}

void CommentRow::mousePressEvent(QMouseEvent *event) {
    if (event->button() == Qt::LeftButton) emit pressed();
    QWidget::mousePressEvent(event);
}

// ------------------------------------------------------------------ ThreadWidget

ThreadWidget::ThreadWidget(const QString &id, bool first, QWidget *parent) : QWidget(parent), m_id(id), m_first(first) {
    setObjectName(QStringLiteral("thread"));
    auto *layout = new QVBoxLayout(this);
    // The rows' bleed is inside the thread's 16 padding: rows start 8 in.
    // Under another thread: its line, then 12 to the content (a row's 6
    // bleed is part of that).
    layout->setContentsMargins(16 - CommentRow::kBleedX, first ? 0 : 12 - CommentRow::kBleedY, 16 - CommentRow::kBleedX, 0);
    layout->setSpacing(0);
    m_replies = new QWidget;
    m_replies->setObjectName(QStringLiteral("replies"));
    m_replyLayout = new QVBoxLayout(m_replies);
    // 12 under the root's content, 36 in (avatar and gap).
    m_replyLayout->setContentsMargins(CommentRow::kBodyLeft, 0, 0, 0);
    m_replyLayout->setSpacing(0);
    layout->addWidget(m_replies);
    m_replies->hide();
}

void ThreadWidget::setFlat(bool flat) {
    m_replyLayout->setContentsMargins(flat ? 0 : CommentRow::kBodyLeft, 0, 0, 0);
}

void ThreadWidget::setRoot(CommentRow *row) {
    m_root = row;
    row->setReply(false);
    static_cast<QVBoxLayout *>(layout())->insertWidget(0, row);
}

void ThreadWidget::addReply(CommentRow *row) {
    row->setReply(true);
    m_replyLayout->addWidget(row);
}

QList<CommentRow *> ThreadWidget::replyRows() const {
    QList<CommentRow *> rows;
    for (int index = 0; index < m_replyLayout->count(); ++index)
        if (auto *row = qobject_cast<CommentRow *>(m_replyLayout->itemAt(index)->widget())) rows.append(row);
    return rows;
}

void ThreadWidget::setRepliesShown(bool shown) { m_replies->setVisible(shown && m_replyLayout->count() > 0); }

bool ThreadWidget::repliesShown() const { return !m_replies->isHidden(); }

QList<CommentRow *> ThreadWidget::rows() const {
    QList<CommentRow *> rows;
    if (m_root) rows.append(m_root);
    rows += replyRows();
    return rows;
}

void ThreadWidget::paintEvent(QPaintEvent *) {
    if (m_first) return;
    QPainter p(this);
    p.fillRect(QRectF(16, 0, width() - 32, 1), color(line));
}

} // namespace kerf
