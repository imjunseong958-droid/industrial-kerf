// Tests for design_tokens.h: the values are the review-thread mockup's
// (mockup-spec 0.1-0.4), and the helpers make what they promise.

#include "design_tokens.h"
#include "test_registry.h"

#include <QColor>
#include <QEasingCurve>
#include <QFont>
#include <QObject>
#include <QSet>
#include <QSettings>
#include <QTest>

#include <initializer_list>
#include <iterator>
#include <utility>

namespace kerf {

class TokensTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    // One graphite ramp and one blue accent, at the mockup's hex values.
    void paletteIsTheMockupsGraphiteRamp() {
        using namespace tokens;
        const std::initializer_list<std::pair<QRgb, const char *>> opaque{
            {surround, "#0e0e0e"}, {surroundDim, "#050505"}, {well, "#121212"},   {chrome, "#171717"},
            {card, "#1c1c1c"},     {raised, "#262626"},      {on, "#303030"},     {hover2, "#363636"},
            {ink, "#ededed"},      {ink2, "#a8a8a8"},        {ink3, "#828282"},   {dis, "#5c5c5c"},
            {track, "#3a3a3a"},    {track2, "#6a6a6a"},      {accent, "#3366e6"}, {accentHover, "#2b5ad6"},
            {accentInk, "#8fb0ff"}, {pos, "#4cc38a"},        {neg, "#ec5d61"},    {playhead, "#e5484d"},
            {warn, "#d6a345"}};
        for (const auto &[value, hex] : opaque) {
            QCOMPARE(color(value), QColor(QString::fromLatin1(hex)));
            QCOMPARE(qAlpha(value), 255);
        }
        // The ramp is neutral grey: no step leans warm or cool.
        for (const QRgb grey : {surround, surroundDim, well, chrome, card, raised, on, hover2, ink, ink2, ink3, dis, track, track2}) {
            QCOMPARE(qRed(grey), qGreen(grey));
            QCOMPARE(qGreen(grey), qBlue(grey));
        }
        // Surfaces step up in lightness from the dimmed stage to the hover.
        const QRgb surfaces[] = {surroundDim, surround, well, chrome, card, raised, on, hover2};
        for (size_t i = 1; i < std::size(surfaces); ++i) QVERIFY(qRed(surfaces[i]) > qRed(surfaces[i - 1]));
    }

    // Hairlines, outlines and the hover outline are white at 8, 14 and 22 %.
    void linesAreWhiteAtTheirAlphas() {
        using namespace tokens;
        const std::initializer_list<std::pair<QRgb, int>> lines{{line, 20}, {line2, 36}, {lineh, 56}};
        for (const auto &[value, alpha] : lines) {
            QCOMPARE(qRgb(qRed(value), qGreen(value), qBlue(value)), qRgb(255, 255, 255));
            QCOMPARE(qAlpha(value), alpha);
        }
        QCOMPARE(color(accentWash), QColor(0x33, 0x66, 0xe6, 36));
        QCOMPARE(qAlpha(pop.color), 115);
        QCOMPARE(pop.dy, 8.0);
        QCOMPARE(pop.blur, 24.0);
        QCOMPARE(color(overlay::captionBackground), QColor(0, 0, 0, 184));
        QCOMPARE(color(overlay::lowerThirdBackground), QColor(12, 12, 12, 199));
    }

    // Only the spec's size/line pairs exist, at 400 or 500.
    void typeScaleHasOnlyTheSpecsPairs() {
        using namespace tokens;
        const QSet<std::pair<int, int>> pairs{{13, 20}, {14, 20}, {12, 16}, {11, 16}, {15, 20}, {11, 12}};
        for (const TypeStyle &style : {body, bodyName, title, control, controlStrong, meta, metaStrong, timecode,
                                       timecodeStrong, monoSmall, monoLarge, caption, badge}) {
            QVERIFY2(pairs.contains({style.px, style.line}), qPrintable(QStringLiteral("%1/%2").arg(style.px).arg(style.line)));
            QVERIFY(style.weight == 400 || style.weight == 500);
        }
        QVERIFY(!body.mono && !title.mono && !caption.mono);
        QVERIFY(timecode.mono && monoSmall.mono && monoLarge.mono && badge.mono);
        QCOMPARE(monoLarge.weight, 500);
        QCOMPARE(caption.px, 15);
    }

    // A style's font is the bundled face first, then the system's, at the
    // style's pixel size and weight, with no letter spacing.
    void fontFollowsItsStyle() {
        using namespace tokens;
        const QFont ui = font(title);
        QCOMPARE(ui.families().value(0), QStringLiteral("Inter"));
        QCOMPARE(ui.pixelSize(), 14);
        QCOMPARE(ui.weight(), QFont::Medium);
        QCOMPARE(ui.letterSpacing(), 0.0);
        const QFont mono = font(timecode);
        QCOMPARE(mono.families().value(0), QStringLiteral("JetBrains Mono"));
        QCOMPARE(mono.families().value(1), QStringLiteral("Menlo"));
        QCOMPARE(mono.pixelSize(), 12);
        QCOMPARE(mono.weight(), QFont::Normal);
#if QT_VERSION >= QT_VERSION_CHECK(6, 7, 0)
        // Figures are tabular everywhere, so timecodes and counts hold still.
        QCOMPARE(ui.featureValue(QFont::Tag("tnum")), 1u);
        QCOMPARE(mono.featureValue(QFont::Tag("tnum")), 1u);
#endif
    }

    // cubic-bezier(.22, 1, .36, 1): fast out of the start, settled at the end.
    void easingStartsFastAndSettles() {
        const QEasingCurve curve = tokens::easing();
        QCOMPARE(curve.valueForProgress(0.0), 0.0);
        QCOMPARE(curve.valueForProgress(1.0), 1.0);
        QVERIFY(curve.valueForProgress(0.25) > 0.6);
        qreal last = 0;
        for (int step = 1; step <= 20; ++step) {
            const qreal value = curve.valueForProgress(step / 20.0);
            QVERIFY(value >= last - 1e-6);
            QVERIFY(value <= 1.0 + 1e-6);
            last = value;
        }
        QVERIFY(tokens::motion::hover < tokens::motion::fade);
        QVERIFY(tokens::motion::fade < tokens::motion::segment);
        QVERIFY(tokens::motion::segment < tokens::motion::move);
    }

    // The default sizes fill the 1600 x 1000 design edge to edge.
    void layoutFillsTheDesignCanvas() {
        using namespace tokens::layout;
        // A wide picture and its conversation over the timeline.
        const int picture = canvasWidth - conversationWide;
        QCOMPARE(picture, 1200);
        QCOMPARE(picture * 9 / 16, 675);
        QCOMPARE(canvasHeight - toolbar - 675, 285);
        // The strip and the ruler over the default tracks' 225.
        QCOMPARE(285 - strip - 32, 225);
        // A vertical picture has the content's height.
        QCOMPARE(canvasHeight - toolbar, 960);
        QCOMPARE(band, 344);
        QVERIFY(conversationLeast < conversationWide);
        QVERIFY(timelineLeast < band);
        QCOMPARE(inspector, browser);
        QVERIFY(bin < inspector);
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::TokensTest)

#include "tokens_test.moc"
